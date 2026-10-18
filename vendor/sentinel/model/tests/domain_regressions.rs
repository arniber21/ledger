//! Domain regressions preserved across the generated-storage migration.

use std::collections::{BTreeMap, BTreeSet};

use sentinel_model::{
    causal_relation, classify_completeness, classify_delivery, classify_delivery_identity,
    classify_lateness, compare_occurrence_times, AttributeDouble, AttributeKey, AttributeValue,
    CausalRelation, ClockDomainId, ClockRelation, CompletenessEvidence, CompletenessFreshness,
    CompletenessSignalError, DataClass, DeliveryIdentityRelation, DeliveryRelation, DeliverySlot,
    Event, EventError, EventId, EventKind, FilterEpoch, Integrity, LateRelation,
    MissingClockCapability, MonitoredApplicationId, ObservationTime, OccurrenceTime,
    ProducerCompletenessSignal, ProducerEvent, SchemaVersion, Sensitivity, SentinelClockId,
    SequenceEpoch, SequenceNumber, SourceActivity, SourceAssurance, SourceCapabilities,
    SourceCapability, SourceCompletenessSignal, SourceId, SourceIdentity, SourceTier,
};

fn source(application: &str, id: &str) -> SourceIdentity {
    SourceIdentity::new(
        MonitoredApplicationId::new(application).expect("valid fixture application"),
        SourceId::new(id).expect("valid fixture source"),
    )
}

fn observation_on(clock: &str, nanoseconds: i128) -> ObservationTime {
    ObservationTime::new(
        SentinelClockId::new(clock).expect("valid fixture Sentinel clock"),
        nanoseconds,
    )
}

fn observation(nanoseconds: i128) -> ObservationTime {
    observation_on("sentinel-a", nanoseconds)
}

#[derive(Clone)]
struct EventFixture {
    id: &'static str,
    source: SourceIdentity,
    epoch: u64,
    sequence: u64,
    schema: &'static str,
    filter_epoch: u64,
    kind: &'static str,
    occurrence: OccurrenceTime,
    observation: ObservationTime,
    attributes: BTreeMap<AttributeKey, AttributeValue>,
    capabilities: SourceCapabilities,
    integrity: Integrity,
    sensitivity: Sensitivity,
    tier: SourceTier,
    predecessors: BTreeSet<EventId>,
}

impl EventFixture {
    fn basic(
        id: &'static str,
        source: SourceIdentity,
        epoch: u64,
        sequence: u64,
        clock: &str,
        predecessors: &[&str],
    ) -> Self {
        Self {
            id,
            source,
            epoch,
            sequence,
            schema: "event.v1",
            filter_epoch: 7,
            kind: "tool.requested",
            occurrence: OccurrenceTime::new(
                ClockDomainId::new(clock).expect("valid fixture producer clock"),
                i128::from(sequence) * 10,
                1,
            ),
            observation: observation(1_000 + i128::from(sequence)),
            attributes: BTreeMap::new(),
            capabilities: SourceCapabilities::new([SourceCapability::BoundedClockUncertainty]),
            integrity: Integrity::Unverified,
            sensitivity: Sensitivity::Internal,
            tier: SourceTier::Contributing,
            predecessors: predecessors
                .iter()
                .map(|id| EventId::new(*id).expect("valid fixture predecessor"))
                .collect(),
        }
    }

    fn build(self) -> Result<Event, EventError> {
        let producer = ProducerEvent::new(
            EventId::new(self.id).expect("valid fixture Event id"),
            SequenceEpoch::new(self.epoch),
            SequenceNumber::new(self.sequence),
            SchemaVersion::new(self.schema).expect("valid fixture schema"),
            FilterEpoch::new(self.filter_epoch),
            EventKind::new(self.kind).expect("valid fixture kind"),
            self.occurrence,
            self.attributes,
            self.capabilities.clone(),
            self.sensitivity,
            self.predecessors,
        )?;
        let assurance = SourceAssurance::new(
            self.source,
            self.tier,
            self.capabilities,
            self.integrity,
            Vec::new(),
            Vec::new(),
            "topology-v1",
        )?;
        Ok(Event::observe(
            producer,
            assurance,
            self.observation,
            DataClass::Tenant,
        ))
    }
}

fn event(
    id: &'static str,
    source: SourceIdentity,
    epoch: u64,
    sequence: u64,
    clock: &str,
    predecessors: &[&str],
) -> Event {
    EventFixture::basic(id, source, epoch, sequence, clock, predecessors)
        .build()
        .expect("valid fixture Event")
}

fn signal(
    source: SourceIdentity,
    epoch: u64,
    last_emitted: Option<u64>,
    complete_through: Option<u64>,
    filter_epoch: u64,
    activity: SourceActivity,
) -> Result<SourceCompletenessSignal, CompletenessSignalError> {
    let producer = ProducerCompletenessSignal::new(
        SequenceEpoch::new(epoch),
        last_emitted.map(SequenceNumber::new),
        complete_through.map(SequenceNumber::new),
        OccurrenceTime::new(
            ClockDomainId::new("producer-a").expect("valid fixture producer clock"),
            100,
            2,
        ),
        FilterEpoch::new(filter_epoch),
        activity,
    )?;
    let assurance = SourceAssurance::new(
        source,
        SourceTier::Contributing,
        SourceCapabilities::new([SourceCapability::CompletenessSignals]),
        Integrity::Authenticated,
        Vec::new(),
        Vec::new(),
        "topology-v1",
    )
    .expect("valid fixture assurance");
    Ok(SourceCompletenessSignal::observe(
        producer,
        assurance,
        observation(1_000),
    ))
}

fn freshness(nanoseconds: i128) -> CompletenessFreshness {
    CompletenessFreshness::observed_not_before(observation(nanoseconds))
}

#[test]
fn canonical_event_preserves_every_contract_field() {
    let event_source = source("payments", "gateway");
    let predecessor = EventId::new("approval-1").expect("valid fixture predecessor");
    let attributes = BTreeMap::from([
        (
            AttributeKey::new("nested").expect("valid fixture"),
            AttributeValue::KeyValueList(BTreeMap::from([
                (
                    AttributeKey::new("a").expect("valid fixture"),
                    AttributeValue::Integer(-1),
                ),
                (
                    AttributeKey::new("z").expect("valid fixture"),
                    AttributeValue::Bool(true),
                ),
            ])),
        ),
        (
            AttributeKey::new("protobuf-shapes").expect("valid fixture"),
            AttributeValue::Array(vec![
                AttributeValue::String("value".to_owned()),
                AttributeValue::Double(AttributeDouble::try_from(1.5).expect("finite double")),
                AttributeValue::Bytes(vec![0, 1, 2]),
            ]),
        ),
    ]);
    let capabilities = SourceCapabilities::new([
        SourceCapability::ObserveBeforeEffect,
        SourceCapability::CausalEdges,
        SourceCapability::CompletenessSignals,
        SourceCapability::BoundedClockUncertainty,
    ]);
    let occurrence = OccurrenceTime::new(
        ClockDomainId::new("gateway-clock").expect("valid fixture producer clock"),
        55,
        3,
    );
    let observed = observation(101);
    let mut fixture = EventFixture::basic(
        "transfer-requested",
        event_source.clone(),
        4,
        9,
        "gateway-clock",
        &[],
    );
    fixture.schema = "event.v2";
    fixture.filter_epoch = 8;
    fixture.kind = "transfer.requested";
    fixture.occurrence = occurrence.clone();
    fixture.observation = observed.clone();
    fixture.attributes = attributes.clone();
    fixture.capabilities = capabilities.clone();
    fixture.integrity = Integrity::TamperEvident;
    fixture.sensitivity = Sensitivity::Restricted;
    fixture.tier = SourceTier::Anchor;
    fixture.predecessors.insert(predecessor.clone());
    let event = fixture.build().expect("valid populated Event");
    let producer = event.producer();

    assert_eq!(event.id().as_str(), "transfer-requested");
    assert_eq!(event.source(), event_source);
    assert_eq!(event.delivery().epoch(), SequenceEpoch::new(4));
    assert_eq!(event.delivery().sequence(), SequenceNumber::new(9));
    assert_eq!(producer.schema_version().as_str(), "event.v2");
    assert_eq!(producer.filter_epoch(), FilterEpoch::new(8));
    assert_eq!(producer.kind().as_str(), "transfer.requested");
    assert_eq!(event.occurrence_time(), occurrence);
    assert_eq!(event.observation_time(), observed);
    assert_eq!(producer.attributes(), attributes);
    assert_eq!(event.capabilities(), capabilities);
    assert_eq!(event.integrity(), Integrity::TamperEvident);
    assert_eq!(producer.sensitivity(), Sensitivity::Restricted);
    assert_eq!(event.source_tier(), SourceTier::Anchor);
    assert_eq!(event.causal_predecessors(), BTreeSet::from([predecessor]));
}

#[test]
fn source_sequences_expose_gaps_without_ordering_unrelated_scopes() {
    let source_a = source("app", "a");
    let first = DeliverySlot::new(
        source_a.clone(),
        SequenceEpoch::new(1),
        SequenceNumber::new(4),
    );
    let later = DeliverySlot::new(
        source_a.clone(),
        SequenceEpoch::new(1),
        SequenceNumber::new(8),
    );
    assert_eq!(
        classify_delivery(&first, &later),
        DeliveryRelation::GapBefore { missing: 3 }
    );
    assert_eq!(
        classify_delivery(&later, &first),
        DeliveryRelation::GapAfter { missing: 3 }
    );
    assert_eq!(
        classify_delivery(
            &first,
            &DeliverySlot::new(source_a, SequenceEpoch::new(2), SequenceNumber::new(1))
        ),
        DeliveryRelation::DifferentEpoch
    );
    assert_eq!(
        classify_delivery(
            &first,
            &DeliverySlot::new(
                source("app", "b"),
                SequenceEpoch::new(1),
                SequenceNumber::new(5)
            )
        ),
        DeliveryRelation::DifferentSource
    );
}

#[test]
fn duplicates_and_conflicts_require_the_same_source_qualified_slot() {
    let source = source("app", "probe");
    let original = event("event-a", source.clone(), 1, 10, "producer-a", &[]);
    let duplicate = event("event-a", source.clone(), 1, 10, "producer-a", &[]);
    let conflict = event("event-b", source.clone(), 1, 10, "producer-a", &[]);
    let reused = event("event-a", source.clone(), 1, 11, "producer-a", &[]);
    let distinct = event("event-c", source, 1, 11, "producer-a", &[]);
    assert_eq!(
        classify_delivery_identity(&original, &duplicate),
        DeliveryIdentityRelation::Duplicate
    );
    assert_eq!(
        classify_delivery_identity(&original, &conflict),
        DeliveryIdentityRelation::SlotConflict
    );
    assert_eq!(
        classify_delivery_identity(&original, &reused),
        DeliveryIdentityRelation::IdentityConflict
    );
    assert_eq!(
        classify_delivery_identity(&original, &distinct),
        DeliveryIdentityRelation::Distinct
    );
}

#[test]
fn duplicate_requires_every_producer_and_assurance_field_to_match() {
    let base = EventFixture::basic("event-a", source("app", "probe"), 1, 10, "producer-a", &[]);
    let original = base.clone().build().expect("valid fixture Event");
    let mut observation_changed = base.clone();
    observation_changed.observation = observation(9_999);
    assert_eq!(
        classify_delivery_identity(
            &original,
            &observation_changed.build().expect("valid Event")
        ),
        DeliveryIdentityRelation::Duplicate
    );

    let mut conflicts = Vec::new();
    let mut changed = base.clone();
    changed.schema = "event.v2";
    conflicts.push(changed);
    let mut changed = base.clone();
    changed.filter_epoch = 8;
    conflicts.push(changed);
    let mut changed = base.clone();
    changed.kind = "tool.completed";
    conflicts.push(changed);
    let mut changed = base.clone();
    changed.occurrence = OccurrenceTime::new(
        ClockDomainId::new("producer-a").expect("valid clock"),
        101,
        1,
    );
    conflicts.push(changed);
    let mut changed = base.clone();
    changed.attributes.insert(
        AttributeKey::new("amount").expect("valid fixture"),
        AttributeValue::Integer(1_000),
    );
    conflicts.push(changed);
    let mut changed = base.clone();
    changed.capabilities = SourceCapabilities::new([
        SourceCapability::BoundedClockUncertainty,
        SourceCapability::CausalEdges,
    ]);
    conflicts.push(changed);
    let mut changed = base.clone();
    changed.integrity = Integrity::Authenticated;
    conflicts.push(changed);
    let mut changed = base.clone();
    changed.sensitivity = Sensitivity::Restricted;
    conflicts.push(changed);
    let mut changed = base.clone();
    changed.tier = SourceTier::Anchor;
    conflicts.push(changed);
    let mut changed = base;
    changed
        .predecessors
        .insert(EventId::new("parent").expect("valid parent"));
    conflicts.push(changed);
    for conflicting in conflicts {
        assert_eq!(
            classify_delivery_identity(&original, &conflicting.build().expect("valid Event")),
            DeliveryIdentityRelation::EvidenceConflict
        );
    }
}

#[test]
fn clock_intervals_and_causal_edges_do_not_invent_a_total_order() {
    let left = event("left", source("app", "probe"), 1, 1, "producer-a", &[]);
    let right = event("right", source("app", "probe"), 1, 2, "producer-a", &[]);
    assert_eq!(
        causal_relation(&left, &right),
        CausalRelation::ConcurrentOrIndirect
    );
    assert_eq!(
        compare_occurrence_times(
            &event("overlap-left", source("app", "left"), 1, 1, "clock", &[]),
            &event("overlap-right", source("app", "right"), 1, 1, "clock", &[])
        ),
        ClockRelation::Overlapping
    );
    assert_eq!(
        compare_occurrence_times(&left, &right),
        ClockRelation::Before
    );
    assert_eq!(
        compare_occurrence_times(
            &left,
            &event("other", source("app", "other"), 1, 1, "other-clock", &[])
        ),
        ClockRelation::DifferentClockDomain
    );
}

#[test]
fn clock_order_requires_bounded_uncertainty_from_both_sources() {
    let qualified = event("qualified", source("app", "qualified"), 1, 2, "clock", &[]);
    let mut unqualified = EventFixture::basic(
        "unqualified",
        source("app", "unqualified"),
        1,
        1,
        "clock",
        &[],
    );
    unqualified.capabilities = SourceCapabilities::default();
    let unqualified = unqualified.build().expect("valid Event");
    let mut other = EventFixture::basic("other", source("app", "other"), 1, 3, "clock", &[]);
    other.capabilities = SourceCapabilities::default();
    let other = other.build().expect("valid Event");
    assert_eq!(
        compare_occurrence_times(&unqualified, &qualified),
        ClockRelation::MissingBoundedClockUncertainty {
            missing: MissingClockCapability::Left
        }
    );
    assert_eq!(
        compare_occurrence_times(&qualified, &unqualified),
        ClockRelation::MissingBoundedClockUncertainty {
            missing: MissingClockCapability::Right
        }
    );
    assert_eq!(
        compare_occurrence_times(&unqualified, &other),
        ClockRelation::MissingBoundedClockUncertainty {
            missing: MissingClockCapability::Both
        }
    );
}

#[test]
fn local_causality_accepts_multiple_parents_and_rejects_only_a_self_edge() {
    let event_source = source("app", "probe");
    let fan_in = event(
        "joined",
        event_source.clone(),
        1,
        3,
        "producer-a",
        &["parent-a", "parent-b"],
    );
    assert_eq!(fan_in.causal_predecessors().len(), 2);
    let invalid = EventFixture::basic("self", event_source, 1, 4, "producer-a", &["self"]).build();
    assert_eq!(invalid, Err(EventError::SelfCausalPredecessor));
}

#[test]
fn pairwise_cycle_is_exposed_for_the_future_trace_aggregate_to_reject() {
    let left = event(
        "left",
        source("app", "probe"),
        1,
        1,
        "producer-a",
        &["right"],
    );
    let right = event(
        "right",
        source("app", "probe"),
        1,
        2,
        "producer-a",
        &["left"],
    );
    assert_eq!(
        causal_relation(&left, &right),
        CausalRelation::ContradictoryCycle
    );
}

#[test]
fn an_empty_signal_is_positive_no_event_evidence_not_missing_evidence() {
    let source = source("app", "probe");
    let empty = signal(
        source.clone(),
        1,
        None,
        None,
        3,
        SourceActivity::Active {
            since: OccurrenceTime::new(
                ClockDomainId::new("producer-a").expect("valid clock"),
                0,
                0,
            ),
        },
    )
    .expect("valid signal");
    assert_eq!(
        classify_completeness(
            None,
            &source,
            SequenceEpoch::new(1),
            FilterEpoch::new(3),
            &freshness(1_000)
        ),
        CompletenessEvidence::NoEvidence
    );
    assert_eq!(
        classify_completeness(
            Some(&empty),
            &source,
            SequenceEpoch::new(1),
            FilterEpoch::new(3),
            &freshness(1_000)
        ),
        CompletenessEvidence::ReportedNoEvents { idle: false }
    );
}

#[test]
fn stale_filter_and_idle_state_remain_visible_in_completeness_evidence() {
    let source = source("app", "probe");
    let idle_since = OccurrenceTime::new(
        ClockDomainId::new("producer-a").expect("valid clock"),
        50,
        2,
    );
    let idle = signal(
        source.clone(),
        4,
        Some(12),
        Some(10),
        6,
        SourceActivity::Idle {
            since: idle_since.clone(),
        },
    )
    .expect("valid signal");
    assert!(idle.activity().is_idle());
    assert_eq!(idle.activity().since(), &idle_since);
    assert_eq!(
        classify_completeness(
            Some(&idle),
            &source,
            SequenceEpoch::new(4),
            FilterEpoch::new(7),
            &freshness(900)
        ),
        CompletenessEvidence::StaleFilter {
            reported: FilterEpoch::new(6),
            required: FilterEpoch::new(7)
        }
    );
    assert_eq!(
        classify_completeness(
            Some(&idle),
            &source,
            SequenceEpoch::new(4),
            FilterEpoch::new(6),
            &freshness(900)
        ),
        CompletenessEvidence::CompleteThrough {
            sequence: SequenceNumber::new(10),
            last_emitted: SequenceNumber::new(12),
            idle: true
        }
    );
}

#[test]
fn malformed_completeness_boundaries_and_clock_domains_are_rejected() {
    let clock = ClockDomainId::new("producer-a").expect("valid clock");
    assert_eq!(
        ProducerCompletenessSignal::new(
            SequenceEpoch::new(1),
            None,
            Some(SequenceNumber::new(0)),
            OccurrenceTime::new(clock.clone(), 100, 0),
            FilterEpoch::new(1),
            SourceActivity::Active {
                since: OccurrenceTime::new(clock.clone(), 0, 0)
            }
        ),
        Err(CompletenessSignalError::CompleteWithoutEmission)
    );
    assert_eq!(
        ProducerCompletenessSignal::new(
            SequenceEpoch::new(1),
            Some(SequenceNumber::new(2)),
            Some(SequenceNumber::new(3)),
            OccurrenceTime::new(clock.clone(), 100, 0),
            FilterEpoch::new(1),
            SourceActivity::Active {
                since: OccurrenceTime::new(clock.clone(), 0, 0)
            }
        ),
        Err(CompletenessSignalError::CompleteBeyondLastEmitted)
    );
    assert_eq!(
        ProducerCompletenessSignal::new(
            SequenceEpoch::new(1),
            None,
            None,
            OccurrenceTime::new(clock, 100, 0),
            FilterEpoch::new(1),
            SourceActivity::Idle {
                since: OccurrenceTime::new(
                    ClockDomainId::new("other-clock").expect("valid clock"),
                    0,
                    0
                )
            }
        ),
        Err(CompletenessSignalError::ActivityClockMismatch)
    );
}

#[test]
fn activity_start_after_its_report_is_rejected_but_uncertainty_overlap_is_valid() {
    let clock = ClockDomainId::new("producer-a").expect("valid clock");
    let build = |since| {
        ProducerCompletenessSignal::new(
            SequenceEpoch::new(1),
            None,
            None,
            OccurrenceTime::new(clock.clone(), 100, 2),
            FilterEpoch::new(1),
            SourceActivity::Idle { since },
        )
    };
    assert_eq!(
        build(OccurrenceTime::new(clock.clone(), 110, 1)),
        Err(CompletenessSignalError::ActivitySinceAfterReport)
    );
    assert!(build(OccurrenceTime::new(clock.clone(), 104, 3)).is_ok());
}

#[test]
fn completeness_rejects_unrepresentable_activity_or_report_intervals() {
    let clock = ClockDomainId::new("producer-a").expect("valid clock");
    assert_eq!(
        ProducerCompletenessSignal::new(
            SequenceEpoch::new(1),
            None,
            None,
            OccurrenceTime::new(clock.clone(), 0, 0),
            FilterEpoch::new(1),
            SourceActivity::Idle {
                since: OccurrenceTime::new(clock.clone(), i128::MAX, 1)
            }
        ),
        Err(CompletenessSignalError::ActivityTimeUnrepresentable)
    );
    assert_eq!(
        ProducerCompletenessSignal::new(
            SequenceEpoch::new(1),
            None,
            None,
            OccurrenceTime::new(clock.clone(), i128::MIN, 1),
            FilterEpoch::new(1),
            SourceActivity::Active {
                since: OccurrenceTime::new(clock, 0, 0)
            }
        ),
        Err(CompletenessSignalError::ActivityTimeUnrepresentable)
    );
}

#[test]
fn stale_active_heartbeat_and_idle_partition_both_yield_no_evidence() {
    let source = source("app", "probe");
    let clock = ClockDomainId::new("producer-a").expect("valid clock");
    let active = signal(
        source.clone(),
        1,
        None,
        None,
        3,
        SourceActivity::Active {
            since: OccurrenceTime::new(clock.clone(), 0, 0),
        },
    )
    .expect("valid signal");
    let idle = signal(
        source.clone(),
        1,
        None,
        None,
        3,
        SourceActivity::Idle {
            since: OccurrenceTime::new(clock, 0, 0),
        },
    )
    .expect("valid signal");
    for stale in [&active, &idle] {
        assert_eq!(
            classify_completeness(
                Some(stale),
                &source,
                SequenceEpoch::new(1),
                FilterEpoch::new(3),
                &freshness(1_001)
            ),
            CompletenessEvidence::NoEvidence
        );
    }
    assert_eq!(
        classify_completeness(
            Some(&active),
            &source,
            SequenceEpoch::new(1),
            FilterEpoch::new(4),
            &freshness(1_001)
        ),
        CompletenessEvidence::NoEvidence
    );
    assert_eq!(
        classify_completeness(
            Some(&idle),
            &source,
            SequenceEpoch::new(1),
            FilterEpoch::new(3),
            &freshness(1_000)
        ),
        CompletenessEvidence::ReportedNoEvents { idle: true }
    );
}

#[test]
fn lateness_requires_signal_observation_to_strictly_precede_event_observation() {
    let source = source("app", "probe");
    let boundary = signal(
        source.clone(),
        2,
        Some(8),
        Some(8),
        7,
        SourceActivity::Active {
            since: OccurrenceTime::new(
                ClockDomainId::new("producer-a").expect("valid clock"),
                0,
                0,
            ),
        },
    )
    .expect("valid signal");
    let late = event("late", source.clone(), 2, 5, "producer-a", &[]);
    let ahead = event("ahead", source.clone(), 2, 9, "producer-a", &[]);
    let old_epoch = event("old-epoch", source.clone(), 1, 5, "producer-a", &[]);
    let wrong_clock = event("wrong-clock", source.clone(), 2, 5, "producer-b", &[]);
    let mut earlier = EventFixture::basic("earlier", source.clone(), 2, 5, "producer-a", &[]);
    earlier.observation = observation(999);
    let mut equal = EventFixture::basic("equal", source.clone(), 2, 5, "producer-a", &[]);
    equal.observation = observation(1_000);
    let mut different = EventFixture::basic("different", source, 2, 5, "producer-a", &[]);
    different.observation = observation_on("sentinel-b", 1_005);
    assert_eq!(
        classify_lateness(&late, &boundary),
        LateRelation::BehindBoundary { positions: 3 }
    );
    assert_eq!(
        classify_lateness(&ahead, &boundary),
        LateRelation::AheadOfBoundary
    );
    assert_eq!(
        classify_lateness(&old_epoch, &boundary),
        LateRelation::DifferentEpoch
    );
    assert_eq!(
        classify_lateness(&wrong_clock, &boundary),
        LateRelation::DifferentClockDomain
    );
    assert_eq!(
        classify_lateness(&earlier.build().expect("valid Event"), &boundary),
        LateRelation::ObservedBeforeSignal
    );
    assert_eq!(
        classify_lateness(&equal.build().expect("valid Event"), &boundary),
        LateRelation::ObservedAtSignal
    );
    assert_eq!(
        classify_lateness(&different.build().expect("valid Event"), &boundary),
        LateRelation::DifferentSentinelClock
    );
}
