//! Boundary tests proving raw generated messages are not canonical domain values.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error as _;

use prost::Message as _;

use sentinel_model::wire::model::v1 as raw;
use sentinel_model::wire::probe::v1 as raw_probe;
use sentinel_model::{
    classify_completeness, AttributeDouble, AttributeKey, AttributeValue, ClockDomainId,
    CompletenessEvidence, CompletenessFreshness, CompletenessSignalError, DataClass, DecideRequest,
    Event, EventError, EventFilter, EventFilterError, EventId, EventKind, FilterEpoch,
    IdentifierError, Integrity, MonitoredApplicationId, ObservationTime, OccurrenceTime,
    ProducerCompletenessSignal, ProducerEvent, SchemaVersion, Sensitivity, SentinelClockId,
    SequenceEpoch, SequenceNumber, SourceActivity, SourceAssurance, SourceCapabilities,
    SourceCapability, SourceCompletenessSignal, SourceId, SourceIdentity, SourceTier, Verdict,
    VerdictError,
};

fn source() -> SourceIdentity {
    SourceIdentity::new(
        MonitoredApplicationId::new("payments").expect("valid fixture"),
        SourceId::new("gateway").expect("valid fixture"),
    )
}

fn occurrence(value: i128) -> OccurrenceTime {
    OccurrenceTime::new(
        ClockDomainId::new("producer-clock").expect("valid fixture"),
        value,
        1,
    )
}

fn observation(value: i128) -> ObservationTime {
    ObservationTime::new(
        SentinelClockId::new("sentinel-clock").expect("valid fixture"),
        value,
    )
}

fn assurance() -> SourceAssurance {
    SourceAssurance::new(
        source(),
        SourceTier::Anchor,
        SourceCapabilities::new([
            SourceCapability::ObserveBeforeEffect,
            SourceCapability::CausalEdges,
            SourceCapability::CompletenessSignals,
            SourceCapability::BoundedClockUncertainty,
        ]),
        Integrity::Authenticated,
        vec![1],
        vec![2],
        "payments-topology",
    )
    .expect("valid assurance")
}

fn producer() -> ProducerEvent {
    let attributes = BTreeMap::from([(
        AttributeKey::new("nested").expect("valid fixture"),
        AttributeValue::KeyValueList(BTreeMap::from([(
            AttributeKey::new("values").expect("valid fixture"),
            AttributeValue::Array(vec![
                AttributeValue::Integer(-1),
                AttributeValue::Double(AttributeDouble::try_from(1.5).expect("finite")),
            ]),
        )])),
    )]);
    ProducerEvent::new(
        EventId::new("transfer-requested").expect("valid fixture"),
        SequenceEpoch::new(2),
        SequenceNumber::new(9),
        SchemaVersion::new("event.v1").expect("valid fixture"),
        FilterEpoch::new(7),
        EventKind::new("transfer.requested").expect("valid fixture"),
        occurrence(10),
        attributes,
        SourceCapabilities::new([SourceCapability::CausalEdges]),
        Sensitivity::Restricted,
        BTreeSet::from([EventId::new("approval").expect("valid fixture")]),
    )
    .expect("valid producer event")
}

fn raw_producer(id: &str) -> raw::ProducerEvent {
    raw::ProducerEvent {
        id: id.to_owned(),
        sequence: Some(raw::SequenceCoordinate::default()),
        schema_version: "v1".to_owned(),
        acknowledged_filter_epoch: Some(0),
        kind: "kind".to_owned(),
        occurrence_time: Some(raw::OccurrenceTime {
            clock_domain_id: "clock".to_owned(),
            nanoseconds: Some(raw::Int128::default()),
            uncertainty_nanoseconds: 0,
        }),
        claimed_sensitivity: raw::Sensitivity::Internal as i32,
        ..raw::ProducerEvent::default()
    }
}

fn raw_source() -> raw::SourceIdentity {
    raw::SourceIdentity {
        monitored_application_id: "payments".to_owned(),
        source_id: "gateway".to_owned(),
    }
}

fn raw_source_named(id: &str) -> raw::SourceIdentity {
    raw::SourceIdentity {
        monitored_application_id: "payments".to_owned(),
        source_id: id.to_owned(),
    }
}

fn raw_source_assurance() -> raw::SourceAssurance {
    raw::SourceAssurance {
        source: Some(raw_source()),
        tier: raw::SourceTier::Anchor as i32,
        effective_capabilities: vec![
            raw::SourceCapability::CompletenessSignals as i32,
            raw::SourceCapability::CausalEdges as i32,
        ],
        integrity: raw::Integrity::Authenticated as i32,
        authenticated_identity_evidence: vec![1],
        integrity_evidence: vec![2],
        topology_id: "topology-v1".to_owned(),
    }
}

fn raw_watermark() -> raw::Watermark {
    raw::Watermark {
        source: Some(raw_source()),
        sequence_epoch: 1,
        complete_through: 4,
        observed_at: Some(raw::ObservationTime {
            sentinel_clock_id: "sentinel".to_owned(),
            nanoseconds: Some(raw::Int128::default()),
        }),
    }
}

fn raw_assurance() -> raw::AssuranceEnvelope {
    raw::AssuranceEnvelope {
        specification_id: "spec".to_owned(),
        specification_version: "v1".to_owned(),
        schema_versions: vec!["event.v1".to_owned()],
        filter_epochs: vec![raw::FilterEpochBySource {
            source: Some(raw_source()),
            filter_epoch: Some(9),
        }],
        watermarks: vec![raw_watermark()],
        required_sources: vec![raw_source()],
        observed_sources: vec![raw_source()],
        gaps: Vec::new(),
        source_health: vec![raw::SourceHealth {
            source: Some(raw_source()),
            healthy: true,
            detail: String::new(),
        }],
        capabilities: vec![raw::SourceCapabilityRecord {
            source: Some(raw_source()),
            capabilities: vec![raw::SourceCapability::CausalEdges as i32],
        }],
        source_tiers_and_anchors: vec![raw::SourceTierRecord {
            source: Some(raw_source()),
            tier: raw::SourceTier::Anchor as i32,
            anchored_event_ids: vec!["event".to_owned()],
        }],
        unresolved_causal_edges: Vec::new(),
        extraction_facts: vec![raw::ExtractionFact {
            fact_id: "amount".to_owned(),
            available: true,
            detail: String::new(),
        }],
        uncertainty: Vec::new(),
        late_data_policy: raw::LateDataPolicy::Exclude as i32,
        hot_path: raw::TierSettlement::Settled as i32,
        cold_path: raw::TierSettlement::NotRequired as i32,
        hot_settlement_watermarks: vec![raw_watermark()],
        cold_settlement_watermarks: Vec::new(),
        source_tier_gap: None,
        settlement_basis: Some(raw::SettlementBasis {
            basis: Some(raw::settlement_basis::Basis::CompleteEvidence(
                raw::CompleteEvidenceBasis {},
            )),
        }),
        activation_barrier: raw::ActivationBarrierStatus::NotDeclared as i32,
        target_filter_epoch: None,
        raw_event_retention_days: Some(30),
        evidence_extract_retention: Some(
            raw::assurance_envelope::EvidenceExtractRetention::EvidenceExtractNotDeployed(
                raw::EvidenceExtractRetentionNotDeployed {},
            ),
        ),
        redaction_policy: Some(
            raw::assurance_envelope::RedactionPolicy::RedactionPolicyNone(
                raw::RedactionPolicyNone {},
            ),
        ),
        redacted_attributes: Vec::new(),
    }
}

fn raw_verdict(assurance: raw::AssuranceEnvelope) -> raw::Verdict {
    raw::Verdict {
        id: "verdict".to_owned(),
        outcome: raw::VerdictOutcome::Satisfy as i32,
        inconclusive_reason: None,
        authority: raw::VerdictAuthority::Authoritative as i32,
        assurance: Some(assurance),
    }
}

#[allow(clippy::too_many_lines)] // Complete fixture intentionally shows every ordered collection.
fn raw_provisional_verdict(reverse: bool) -> raw::Verdict {
    let second_source = raw_source_named("ledger");
    let mut second_watermark = raw_watermark();
    second_watermark.source = Some(second_source.clone());
    let gap = |source, first, effect: &str| raw::SourceGap {
        source: Some(source),
        sequence_epoch: 1,
        first_missing: first,
        last_missing: first,
        effect: effect.to_owned(),
    };
    let mut assurance = raw::AssuranceEnvelope {
        specification_id: "spec".to_owned(),
        specification_version: "v1".to_owned(),
        schema_versions: vec!["event.v2".to_owned(), "event.v1".to_owned()],
        filter_epochs: vec![
            raw::FilterEpochBySource {
                source: Some(second_source.clone()),
                filter_epoch: Some(9),
            },
            raw::FilterEpochBySource {
                source: Some(raw_source()),
                filter_epoch: Some(9),
            },
        ],
        watermarks: vec![second_watermark.clone(), raw_watermark()],
        required_sources: vec![second_source.clone(), raw_source()],
        observed_sources: vec![second_source.clone(), raw_source()],
        gaps: vec![
            gap(second_source.clone(), 3, "ledger evidence missing"),
            gap(raw_source(), 2, "gateway evidence missing"),
        ],
        source_health: vec![
            raw::SourceHealth {
                source: Some(second_source.clone()),
                healthy: false,
                detail: "ledger heartbeat expired".to_owned(),
            },
            raw::SourceHealth {
                source: Some(raw_source()),
                healthy: true,
                detail: String::new(),
            },
        ],
        capabilities: vec![
            raw::SourceCapabilityRecord {
                source: Some(second_source.clone()),
                capabilities: vec![
                    raw::SourceCapability::CompletenessSignals as i32,
                    raw::SourceCapability::CausalEdges as i32,
                ],
            },
            raw::SourceCapabilityRecord {
                source: Some(raw_source()),
                capabilities: vec![
                    raw::SourceCapability::ObserveBeforeEffect as i32,
                    raw::SourceCapability::CausalEdges as i32,
                ],
            },
        ],
        source_tiers_and_anchors: vec![
            raw::SourceTierRecord {
                source: Some(second_source),
                tier: raw::SourceTier::Contributing as i32,
                anchored_event_ids: Vec::new(),
            },
            raw::SourceTierRecord {
                source: Some(raw_source()),
                tier: raw::SourceTier::Anchor as i32,
                anchored_event_ids: vec!["event-b".to_owned(), "event-a".to_owned()],
            },
        ],
        unresolved_causal_edges: vec![
            raw::UnresolvedCausalEdge {
                event_id: "z-event".to_owned(),
                missing_predecessor_id: "z-parent".to_owned(),
            },
            raw::UnresolvedCausalEdge {
                event_id: "a-event".to_owned(),
                missing_predecessor_id: "a-parent".to_owned(),
            },
        ],
        extraction_facts: vec![
            raw::ExtractionFact {
                fact_id: "z-fact".to_owned(),
                available: false,
                detail: "opaque".to_owned(),
            },
            raw::ExtractionFact {
                fact_id: "a-fact".to_owned(),
                available: true,
                detail: String::new(),
            },
        ],
        uncertainty: vec![
            raw::Uncertainty {
                reason: raw::InconclusiveReason::PrefixUncertainty as i32,
                detail: "gap".to_owned(),
            },
            raw::Uncertainty {
                reason: raw::InconclusiveReason::EpistemicUncertainty as i32,
                detail: "unknown".to_owned(),
            },
        ],
        late_data_policy: raw::LateDataPolicy::Exclude as i32,
        hot_path: raw::TierSettlement::Settled as i32,
        cold_path: raw::TierSettlement::Pending as i32,
        hot_settlement_watermarks: vec![second_watermark, raw_watermark()],
        cold_settlement_watermarks: Vec::new(),
        source_tier_gap: None,
        settlement_basis: None,
        activation_barrier: raw::ActivationBarrierStatus::NotDeclared as i32,
        target_filter_epoch: None,
        raw_event_retention_days: Some(30),
        evidence_extract_retention: Some(
            raw::assurance_envelope::EvidenceExtractRetention::EvidenceExtractNotDeployed(
                raw::EvidenceExtractRetentionNotDeployed {},
            ),
        ),
        redaction_policy: Some(
            raw::assurance_envelope::RedactionPolicy::RedactionPolicyNone(
                raw::RedactionPolicyNone {},
            ),
        ),
        redacted_attributes: Vec::new(),
    };
    if reverse {
        assurance.schema_versions.reverse();
        assurance.filter_epochs.reverse();
        assurance.watermarks.reverse();
        assurance.required_sources.reverse();
        assurance.observed_sources.reverse();
        assurance.gaps.reverse();
        assurance.source_health.reverse();
        assurance.capabilities.reverse();
        for record in &mut assurance.capabilities {
            record.capabilities.reverse();
        }
        assurance.source_tiers_and_anchors.reverse();
        for record in &mut assurance.source_tiers_and_anchors {
            record.anchored_event_ids.reverse();
        }
        assurance.unresolved_causal_edges.reverse();
        assurance.extraction_facts.reverse();
        assurance.uncertainty.reverse();
        assurance.hot_settlement_watermarks.reverse();
    }
    raw::Verdict {
        id: "provisional".to_owned(),
        outcome: raw::VerdictOutcome::Inconclusive as i32,
        inconclusive_reason: Some(raw::InconclusiveReason::PrefixUncertainty as i32),
        authority: raw::VerdictAuthority::Provisional as i32,
        assurance: Some(assurance),
    }
}

#[test]
fn generated_event_roundtrips_through_validation() {
    let event = Event::observe(producer(), assurance(), observation(20), DataClass::Tenant);
    let decoded = Event::decode(&event.encode_to_vec()).expect("encoded canonical Event validates");
    assert_eq!(decoded, event);
    assert_eq!(decoded.id().as_str(), "transfer-requested");
    assert_eq!(decoded.source(), source());
    assert_eq!(decoded.delivery().sequence(), SequenceNumber::new(9));
    assert_eq!(decoded.producer().attributes().len(), 1);
}

fn nested_attribute_entries(reverse: bool, negative_zero: bool) -> Vec<raw::AttributeEntry> {
    let zero = if negative_zero { -0.0 } else { 0.0 };
    let mut nested = vec![
        raw::AttributeEntry {
            key: "beta".to_owned(),
            value: Some(raw::AttributeValue {
                value: Some(raw::attribute_value::Value::DoubleValue(zero)),
            }),
            claimed_sensitivity: raw::Sensitivity::Unspecified as i32,
        },
        raw::AttributeEntry {
            key: "alpha".to_owned(),
            value: Some(raw::AttributeValue {
                value: Some(raw::attribute_value::Value::IntegerValue(7)),
            }),
            claimed_sensitivity: raw::Sensitivity::Unspecified as i32,
        },
    ];
    if reverse {
        nested.reverse();
    }
    vec![
        raw::AttributeEntry {
            key: "zeta".to_owned(),
            value: Some(raw::AttributeValue {
                value: Some(raw::attribute_value::Value::BoolValue(true)),
            }),
            claimed_sensitivity: raw::Sensitivity::Unspecified as i32,
        },
        raw::AttributeEntry {
            key: "nested-array".to_owned(),
            value: Some(raw::AttributeValue {
                value: Some(raw::attribute_value::Value::ArrayValue(
                    raw::AttributeArray {
                        values: vec![raw::AttributeValue {
                            value: Some(raw::attribute_value::Value::MapValue(raw::AttributeMap {
                                entries: nested,
                            })),
                        }],
                    },
                )),
            }),
            claimed_sensitivity: raw::Sensitivity::Unspecified as i32,
        },
        raw::AttributeEntry {
            key: "alpha".to_owned(),
            value: Some(raw::AttributeValue {
                value: Some(raw::attribute_value::Value::StringValue("value".to_owned())),
            }),
            claimed_sensitivity: raw::Sensitivity::Unspecified as i32,
        },
    ]
}

fn permuted_raw_event(order: &[usize], reverse_sets: bool, negative_zero: bool) -> raw::Event {
    let attributes = nested_attribute_entries(reverse_sets, negative_zero);
    let mut producer = raw_producer("canonical-event");
    producer.attributes = order
        .iter()
        .map(|&index| attributes[index].clone())
        .collect();
    producer.claimed_capabilities = vec![
        raw::SourceCapability::ObserveBeforeEffect as i32,
        raw::SourceCapability::CausalEdges as i32,
    ];
    producer.causal_predecessor_ids = vec!["parent-b".to_owned(), "parent-a".to_owned()];
    let mut assurance = raw_source_assurance();
    if reverse_sets {
        producer.claimed_capabilities.reverse();
        producer.causal_predecessor_ids.reverse();
        assurance.effective_capabilities.reverse();
    }
    raw::Event {
        producer_event: Some(producer),
        assurance: Some(assurance),
        observation_time: Some(raw::ObservationTime {
            sentinel_clock_id: "sentinel".to_owned(),
            nanoseconds: Some(raw::Int128::default()),
        }),
        tenant_id: String::new(),
        deployment_id: String::new(),
        data_class: raw::DataClass::Tenant as i32,
    }
}

#[test]
fn valid_event_permutations_have_canonical_storage_equality_and_encoding() {
    let permutations = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let canonical = Event::try_from(permuted_raw_event(&permutations[0], false, false))
        .expect("baseline raw Event is valid");
    for (index, permutation) in permutations.iter().enumerate() {
        let event = Event::try_from(permuted_raw_event(
            permutation,
            index % 2 == 0,
            index % 2 == 1,
        ))
        .expect("permuted raw Event is valid");
        assert_eq!(event, canonical, "permutation {permutation:?}");
        assert_eq!(event.encode_to_vec(), canonical.encode_to_vec());
    }
}

#[test]
fn signed_zero_is_normalized_but_nonfinite_attributes_remain_invalid() {
    let negative = AttributeDouble::try_from(-0.0).expect("negative zero is finite");
    let positive = AttributeDouble::try_from(0.0).expect("positive zero is finite");
    assert_eq!(negative, positive);
    assert_eq!(negative.value().to_bits(), 0.0_f64.to_bits());

    let event = Event::try_from(permuted_raw_event(&[0, 1, 2], false, true))
        .expect("negative zero Event is valid");
    let decoded = Event::decode(&event.encode_to_vec()).expect("canonical encoding validates");
    assert_eq!(decoded, event);
}

fn shadow_specification(id: &str) -> raw::SpecificationFilter {
    let mut second_watermark = raw_watermark();
    second_watermark.source = Some(raw_source_named("ledger"));
    raw::SpecificationFilter {
        specification_id: id.to_owned(),
        specification_version: "v1".to_owned(),
        event_match: Some(raw::EventMatch {
            event_kinds: vec!["z.kind".to_owned(), "a.kind".to_owned()],
            projected_attribute_keys: vec!["z".to_owned(), "a".to_owned()],
            delivery_mode: raw::DeliveryMode::ShipAsync as i32,
        }),
        fail_mode: raw::FailMode::Open as i32,
        latency_budget_nanoseconds: None,
        evaluation_mode: raw::EvaluationMode::Shadow as i32,
        readiness: raw::Readiness::Warming as i32,
        target_filter_epoch: Some(9),
        required_sources: vec![raw_source_named("ledger"), raw_source()],
        activation_watermarks: vec![second_watermark, raw_watermark()],
        enforcement_gates: None,
    }
}

fn raw_filter(reverse: bool) -> raw::EventFilter {
    let mut specifications = vec![
        shadow_specification("z-specification"),
        shadow_specification("a-specification"),
    ];
    if reverse {
        specifications.reverse();
        for specification in &mut specifications {
            let event_match = specification.event_match.as_mut().expect("fixture match");
            event_match.event_kinds.reverse();
            event_match.projected_attribute_keys.reverse();
            specification.required_sources.reverse();
            specification.activation_watermarks.reverse();
        }
    }
    raw::EventFilter {
        epoch: Some(9),
        specifications,
        baseline_evidence: Some(raw::BaselineEvidenceFilter {
            source_health: true,
            completeness_signals: true,
        }),
    }
}

#[test]
fn filter_sets_are_canonical_for_equality_and_encoding() {
    let forward = EventFilter::try_from(raw_filter(false)).expect("forward Filter is valid");
    let reverse = EventFilter::try_from(raw_filter(true)).expect("reverse Filter is valid");
    assert_eq!(forward, reverse);
    assert_eq!(forward.encode_to_vec(), reverse.encode_to_vec());

    let mut duplicate = raw_filter(false);
    duplicate.specifications[0]
        .required_sources
        .push(raw_source());
    assert_eq!(
        EventFilter::try_from(duplicate),
        Err(EventFilterError::DuplicateValue("required_sources"))
    );
}

#[test]
fn assurance_collections_are_canonical_for_equality_and_encoding() {
    let forward = Verdict::try_from(raw_provisional_verdict(false))
        .expect("forward provisional Verdict is valid");
    let reverse = Verdict::try_from(raw_provisional_verdict(true))
        .expect("reverse provisional Verdict is valid");
    assert_eq!(forward, reverse);
    assert_eq!(forward.encode_to_vec(), reverse.encode_to_vec());
}

#[test]
fn missing_messages_and_unknown_or_unspecified_enums_are_rejected() {
    assert_eq!(
        Event::try_from(raw::Event::default()),
        Err(EventError::MissingField("event.producer_event"))
    );

    let mut raw_producer = raw_producer("id");
    raw_producer.acknowledged_filter_epoch = None;
    assert_eq!(
        ProducerEvent::try_from(raw_producer.clone()),
        Err(EventError::MissingScalar(
            "producer_event.acknowledged_filter_epoch"
        ))
    );
    raw_producer.acknowledged_filter_epoch = Some(0);
    raw_producer.claimed_sensitivity = 99;
    assert_eq!(
        ProducerEvent::try_from(raw_producer.clone()),
        Err(EventError::UnknownEnum {
            field: "sensitivity",
            value: 99
        })
    );
    raw_producer.claimed_sensitivity = raw::Sensitivity::Unspecified as i32;
    assert_eq!(
        ProducerEvent::try_from(raw_producer),
        Err(EventError::UnspecifiedEnum("sensitivity"))
    );
}

#[test]
fn classless_and_unknown_data_classes_are_rejected() {
    let event = Event::observe(producer(), assurance(), observation(20), DataClass::Tenant);
    let mut unspecified = event.encode_to_vec();
    // A classless Event is the silent-wrong failure ADR-0029 exists to prevent:
    // an Event that is neither Tenant nor Corpus can be stored under either
    // regime without anyone noticing, so the model refuses it outright.
    let mut raw_event = raw::Event::decode(unspecified.as_slice()).expect("decodable");
    raw_event.data_class = raw::DataClass::Unspecified as i32;
    unspecified = raw_event.encode_to_vec();
    assert_eq!(
        Event::decode(&unspecified).expect_err("classless Event is rejected"),
        EventError::UnspecifiedEnum("data_class")
    );

    let unknown = {
        let mut raw_event = raw::Event::decode(event.encode_to_vec().as_slice())
            .expect("decodable")
            .clone();
        raw_event.data_class = 99;
        raw_event.encode_to_vec()
    };
    assert_eq!(
        Event::decode(&unknown).expect_err("unknown data class is rejected"),
        EventError::UnknownEnum {
            field: "data_class",
            value: 99
        }
    );

    // Both named classes validate and survive a round trip.
    for class in [DataClass::Tenant, DataClass::Corpus] {
        let event = Event::observe(producer(), assurance(), observation(21), class);
        let decoded = Event::decode(&event.encode_to_vec()).expect("named class validates");
        assert_eq!(decoded.data_class(), class);
    }
}

#[test]
#[allow(clippy::too_many_lines)] // One adversarial fixture covers blank keys, duplicates, nonfinite values, and self-edges together.
fn malformed_attributes_duplicates_nonfinite_values_and_self_edges_are_rejected() {
    let mut raw_producer = raw_producer("self");
    raw_producer.attributes = vec![raw::AttributeEntry {
        key: "x".to_owned(),
        value: Some(raw::AttributeValue { value: None }),
        claimed_sensitivity: raw::Sensitivity::Unspecified as i32,
    }];
    assert_eq!(
        ProducerEvent::try_from(raw_producer.clone()),
        Err(EventError::MissingField("attribute.value"))
    );

    raw_producer.attributes = vec![raw::AttributeEntry {
        key: "   ".to_owned(),
        value: Some(raw::AttributeValue {
            value: Some(raw::attribute_value::Value::BoolValue(true)),
        }),
        claimed_sensitivity: raw::Sensitivity::Unspecified as i32,
    }];
    assert_eq!(
        ProducerEvent::try_from(raw_producer.clone()),
        Err(EventError::InvalidIdentifier {
            field: "attribute.key",
            source: IdentifierError,
        })
    );

    let nan = raw::AttributeValue {
        value: Some(raw::attribute_value::Value::DoubleValue(f64::NAN)),
    };
    raw_producer.attributes = vec![raw::AttributeEntry {
        key: "x".to_owned(),
        value: Some(nan.clone()),
        claimed_sensitivity: raw::Sensitivity::Unspecified as i32,
    }];
    assert_eq!(
        ProducerEvent::try_from(raw_producer.clone()),
        Err(EventError::NonFiniteAttributeDouble)
    );

    let valid = raw::AttributeValue {
        value: Some(raw::attribute_value::Value::BoolValue(true)),
    };
    raw_producer.attributes = vec![
        raw::AttributeEntry {
            key: "x".to_owned(),
            value: Some(valid.clone()),
            claimed_sensitivity: raw::Sensitivity::Unspecified as i32,
        },
        raw::AttributeEntry {
            key: "x".to_owned(),
            value: Some(valid),
            claimed_sensitivity: raw::Sensitivity::Unspecified as i32,
        },
    ];
    assert_eq!(
        ProducerEvent::try_from(raw_producer.clone()),
        Err(EventError::DuplicateAttributeKey)
    );

    let duplicate_nested = raw::AttributeValue {
        value: Some(raw::attribute_value::Value::ArrayValue(
            raw::AttributeArray {
                values: vec![raw::AttributeValue {
                    value: Some(raw::attribute_value::Value::MapValue(raw::AttributeMap {
                        entries: vec![
                            raw::AttributeEntry {
                                key: "nested".to_owned(),
                                value: Some(raw::AttributeValue {
                                    value: Some(raw::attribute_value::Value::BoolValue(true)),
                                }),
                                claimed_sensitivity: raw::Sensitivity::Unspecified as i32,
                            },
                            raw::AttributeEntry {
                                key: "nested".to_owned(),
                                value: Some(raw::AttributeValue {
                                    value: Some(raw::attribute_value::Value::BoolValue(false)),
                                }),
                                claimed_sensitivity: raw::Sensitivity::Unspecified as i32,
                            },
                        ],
                    })),
                }],
            },
        )),
    };
    raw_producer.attributes = vec![raw::AttributeEntry {
        key: "array".to_owned(),
        value: Some(duplicate_nested),
        claimed_sensitivity: raw::Sensitivity::Unspecified as i32,
    }];
    assert_eq!(
        ProducerEvent::try_from(raw_producer.clone()),
        Err(EventError::DuplicateAttributeKey)
    );

    raw_producer.attributes.clear();
    raw_producer.claimed_capabilities = vec![
        raw::SourceCapability::CausalEdges as i32,
        raw::SourceCapability::CausalEdges as i32,
    ];
    assert_eq!(
        ProducerEvent::try_from(raw_producer.clone()),
        Err(EventError::DuplicateValue("claimed_capabilities"))
    );

    raw_producer.claimed_capabilities.clear();
    raw_producer.causal_predecessor_ids = vec!["self".to_owned()];
    assert_eq!(
        ProducerEvent::try_from(raw_producer),
        Err(EventError::SelfCausalPredecessor)
    );
}

#[test]
fn completeness_separates_producer_and_receiver_authority() {
    let producer_signal = ProducerCompletenessSignal::new(
        SequenceEpoch::new(2),
        Some(SequenceNumber::new(9)),
        Some(SequenceNumber::new(8)),
        occurrence(30),
        FilterEpoch::new(7),
        SourceActivity::Active {
            since: occurrence(0),
        },
    )
    .expect("valid producer completeness");
    let signal = SourceCompletenessSignal::observe(producer_signal, assurance(), observation(40));
    assert_eq!(
        classify_completeness(
            Some(&signal),
            &source(),
            SequenceEpoch::new(2),
            FilterEpoch::new(7),
            &CompletenessFreshness::observed_not_before(observation(39)),
        ),
        CompletenessEvidence::CompleteThrough {
            sequence: SequenceNumber::new(8),
            last_emitted: SequenceNumber::new(9),
            idle: false,
        }
    );
}

#[test]
fn completeness_rejects_inconsistent_sequence_bounds() {
    assert!(ProducerCompletenessSignal::new(
        SequenceEpoch::new(2),
        Some(SequenceNumber::new(8)),
        Some(SequenceNumber::new(9)),
        occurrence(30),
        FilterEpoch::new(7),
        SourceActivity::Active {
            since: occurrence(0)
        },
    )
    .is_err());
}

fn raw_producer_completeness() -> raw::ProducerCompletenessSignal {
    raw::ProducerCompletenessSignal {
        sequence_epoch: 1,
        last_emitted: None,
        complete_through: None,
        producer_reported_at: Some(raw::OccurrenceTime {
            clock_domain_id: "producer-clock".to_owned(),
            nanoseconds: Some(raw::Int128::default()),
            uncertainty_nanoseconds: 0,
        }),
        acknowledged_filter_epoch: Some(1),
        activity: raw::SourceActivityState::Active as i32,
        activity_since: Some(raw::OccurrenceTime {
            clock_domain_id: "producer-clock".to_owned(),
            nanoseconds: Some(raw::Int128::default()),
            uncertainty_nanoseconds: 0,
        }),
    }
}

#[test]
fn nested_validation_errors_preserve_specific_variants_and_source_chains() {
    let mut producer_signal = raw_producer_completeness();
    producer_signal
        .producer_reported_at
        .as_mut()
        .expect("fixture producer time")
        .clock_domain_id = " ".to_owned();
    let error = ProducerCompletenessSignal::try_from(producer_signal)
        .expect_err("invalid producer occurrence time must fail");
    assert!(matches!(
        &error,
        CompletenessSignalError::InvalidOccurrence(EventError::InvalidIdentifier {
            field: "occurrence_time.clock_domain_id",
            ..
        })
    ));
    let event_error = error
        .source()
        .expect("completeness error preserves EventError")
        .downcast_ref::<EventError>()
        .expect("first source is EventError");
    assert!(event_error
        .source()
        .expect("EventError preserves IdentifierError")
        .downcast_ref::<IdentifierError>()
        .is_some());

    let mut invalid_assurance = raw_source_assurance();
    invalid_assurance
        .source
        .as_mut()
        .expect("fixture source")
        .source_id = String::new();
    let completeness_error = SourceCompletenessSignal::try_from(raw::SourceCompletenessSignal {
        producer_signal: Some(raw_producer_completeness()),
        assurance: Some(invalid_assurance),
        observed_at: Some(raw::ObservationTime {
            sentinel_clock_id: "sentinel".to_owned(),
            nanoseconds: Some(raw::Int128::default()),
        }),
    })
    .expect_err("invalid assurance must fail");
    assert!(matches!(
        &completeness_error,
        CompletenessSignalError::InvalidAssurance(_)
    ));
    assert!(completeness_error.source().is_some());

    let mut invalid_producer = raw_producer(" ");
    invalid_producer.acknowledged_filter_epoch = Some(1);
    let decide_error = DecideRequest::try_from(raw_probe::DecideRequest {
        request_id: "request".to_owned(),
        idempotency_key: "idempotency".to_owned(),
        source_handle: "source".to_owned(),
        filter_epoch: Some(1),
        producer_event: Some(invalid_producer),
        remaining_transport_budget_nanoseconds: Some(1),
    })
    .expect_err("invalid producer id must fail");
    let event_error = decide_error
        .source()
        .expect("DecideRequestError preserves EventError")
        .downcast_ref::<EventError>()
        .expect("DecideRequestError source is EventError");
    assert!(event_error.source().is_some());
}

#[test]
fn contradictory_authoritative_satisfy_fixture_is_rejected_at_public_boundary() {
    let mut envelope = raw_assurance();
    envelope.source_health[0].healthy = false;
    envelope.source_health[0].detail = "heartbeat expired".to_owned();
    envelope.gaps.push(raw::SourceGap {
        source: Some(raw_source()),
        sequence_epoch: 1,
        first_missing: 2,
        last_missing: 2,
        effect: "can hide a violation".to_owned(),
    });
    envelope
        .unresolved_causal_edges
        .push(raw::UnresolvedCausalEdge {
            event_id: "later".to_owned(),
            missing_predecessor_id: "earlier".to_owned(),
        });
    envelope.extraction_facts[0].available = false;
    envelope.extraction_facts[0].detail = "payload was opaque".to_owned();
    envelope.uncertainty.push(raw::Uncertainty {
        reason: raw::InconclusiveReason::PrefixUncertainty as i32,
        detail: "required evidence is missing".to_owned(),
    });

    assert_eq!(
        Verdict::try_from(raw_verdict(envelope)),
        Err(VerdictError::AuthoritativeEvidenceIncomplete(
            "assurance.source_health"
        ))
    );
}

#[test]
#[allow(clippy::too_many_lines)] // Proof and mirrored limitations stay adjacent for auditability.
fn anchored_decisive_violation_allows_only_explicitly_invariant_limitations() {
    let mut envelope = raw_assurance();
    envelope.source_tiers_and_anchors[0]
        .anchored_event_ids
        .push("event-2".to_owned());
    envelope.gaps.push(raw::SourceGap {
        source: Some(raw_source()),
        sequence_epoch: 1,
        first_missing: 2,
        last_missing: 2,
        effect: "irrelevant to existence witness".to_owned(),
    });
    envelope.gaps.push(raw::SourceGap {
        source: Some(raw_source()),
        sequence_epoch: 1,
        first_missing: 1,
        last_missing: 1,
        effect: "also irrelevant to existence witness".to_owned(),
    });
    envelope
        .unresolved_causal_edges
        .push(raw::UnresolvedCausalEdge {
            event_id: "unrelated".to_owned(),
            missing_predecessor_id: "unknown".to_owned(),
        });
    envelope
        .unresolved_causal_edges
        .push(raw::UnresolvedCausalEdge {
            event_id: "another-unrelated".to_owned(),
            missing_predecessor_id: "another-unknown".to_owned(),
        });
    envelope.extraction_facts.push(raw::ExtractionFact {
        fact_id: "memo".to_owned(),
        available: false,
        detail: "not used by the witness".to_owned(),
    });
    envelope.extraction_facts.push(raw::ExtractionFact {
        fact_id: "note".to_owned(),
        available: false,
        detail: "also not used by the witness".to_owned(),
    });
    envelope.extraction_facts.push(raw::ExtractionFact {
        fact_id: "currency".to_owned(),
        available: true,
        detail: String::new(),
    });
    envelope.uncertainty.push(raw::Uncertainty {
        reason: raw::InconclusiveReason::PrefixUncertainty as i32,
        detail: "the proof rule establishes outcome invariance".to_owned(),
    });
    envelope.uncertainty.push(raw::Uncertainty {
        reason: raw::InconclusiveReason::EpistemicUncertainty as i32,
        detail: "also proven invariant".to_owned(),
    });
    envelope.settlement_basis = Some(raw::SettlementBasis {
        basis: Some(raw::settlement_basis::Basis::DecisiveViolation(
            raw::DecisiveViolationBasis {
                witness_events: vec![
                    raw::DecisiveEvent {
                        event_id: "event".to_owned(),
                        source: Some(raw_source()),
                        sequence_epoch: Some(1),
                        sequence_number: Some(4),
                    },
                    raw::DecisiveEvent {
                        event_id: "event-2".to_owned(),
                        source: Some(raw_source()),
                        sequence_epoch: Some(1),
                        sequence_number: Some(3),
                    },
                ],
                required_extraction_fact_ids: vec!["amount".to_owned(), "currency".to_owned()],
                invariance_proof_rule_id: "existence-witness".to_owned(),
                invariance_proof_rule_version: "v1".to_owned(),
                outcome_invariant_limitations: Some(raw::OutcomeInvariantLimitations {
                    gaps: envelope.gaps.clone(),
                    unhealthy_sources: Vec::new(),
                    unresolved_causal_edges: envelope.unresolved_causal_edges.clone(),
                    unavailable_extraction_fact_ids: vec!["memo".to_owned(), "note".to_owned()],
                    uncertainty: vec![
                        raw::InconclusiveReason::PrefixUncertainty as i32,
                        raw::InconclusiveReason::EpistemicUncertainty as i32,
                    ],
                }),
            },
        )),
    });
    let mut violation = raw_verdict(envelope.clone());
    violation.outcome = raw::VerdictOutcome::Violate as i32;
    let canonical =
        Verdict::try_from(violation.clone()).expect("closed decisive violation basis is valid");
    let assurance = violation.assurance.as_mut().expect("fixture assurance");
    assurance.gaps.reverse();
    assurance.unresolved_causal_edges.reverse();
    assurance.extraction_facts.reverse();
    assurance.uncertainty.reverse();
    assurance.source_tiers_and_anchors[0]
        .anchored_event_ids
        .reverse();
    let Some(raw::settlement_basis::Basis::DecisiveViolation(basis)) = assurance
        .settlement_basis
        .as_mut()
        .and_then(|settlement| settlement.basis.as_mut())
    else {
        panic!("fixture must contain a decisive basis");
    };
    basis.witness_events.reverse();
    basis.required_extraction_fact_ids.reverse();
    let limitations = basis
        .outcome_invariant_limitations
        .as_mut()
        .expect("fixture limitations");
    limitations.gaps.reverse();
    limitations.unresolved_causal_edges.reverse();
    limitations.unavailable_extraction_fact_ids.reverse();
    limitations.uncertainty.reverse();
    let permuted = Verdict::try_from(violation).expect("permuted decisive basis is valid");
    assert_eq!(canonical, permuted);
    assert_eq!(canonical.encode_to_vec(), permuted.encode_to_vec());

    let Some(raw::settlement_basis::Basis::DecisiveViolation(basis)) = envelope
        .settlement_basis
        .as_mut()
        .and_then(|settlement| settlement.basis.as_mut())
    else {
        panic!("fixture must contain a decisive basis");
    };
    basis
        .outcome_invariant_limitations
        .as_mut()
        .expect("fixture limitations")
        .gaps
        .clear();
    let mut unlisted = raw_verdict(envelope);
    unlisted.outcome = raw::VerdictOutcome::Violate as i32;
    assert_eq!(
        Verdict::try_from(unlisted),
        Err(VerdictError::InvariantLimitationsMismatch("gaps"))
    );
}

#[test]
fn provisional_and_inconclusive_public_boundaries_keep_named_gaps() {
    let mut provisional_envelope = raw_assurance();
    provisional_envelope.source_tiers_and_anchors[0].tier = raw::SourceTier::Contributing as i32;
    provisional_envelope.source_tiers_and_anchors[0]
        .anchored_event_ids
        .clear();
    provisional_envelope.source_tier_gap = Some(raw::SourceTierGap {
        contributing_sources: vec![raw_source()],
        detail: "only application-owned evidence is available".to_owned(),
        unanchored_sources: vec![raw_source()],
    });
    provisional_envelope.cold_path = raw::TierSettlement::Pending as i32;
    provisional_envelope.settlement_basis = None;
    let mut provisional = raw_verdict(provisional_envelope);
    provisional.authority = raw::VerdictAuthority::Provisional as i32;
    Verdict::try_from(provisional).expect("typed source-tier gap stays provisional");

    let mut inconclusive_envelope = raw_assurance();
    inconclusive_envelope.gaps.push(raw::SourceGap {
        source: Some(raw_source()),
        sequence_epoch: 1,
        first_missing: 2,
        last_missing: 2,
        effect: "may contain required evidence".to_owned(),
    });
    inconclusive_envelope.uncertainty.push(raw::Uncertainty {
        reason: raw::InconclusiveReason::PrefixUncertainty as i32,
        detail: "the gap prevents settlement".to_owned(),
    });
    inconclusive_envelope.settlement_basis = None;
    let mut inconclusive = raw_verdict(inconclusive_envelope);
    inconclusive.outcome = raw::VerdictOutcome::Inconclusive as i32;
    inconclusive.inconclusive_reason = Some(raw::InconclusiveReason::PrefixUncertainty as i32);
    Verdict::try_from(inconclusive).expect("prefix uncertainty stays inconclusive");
}

/// A fully covered two-required-source envelope whose second anchor-tier source
/// anchored no Event for this Verdict (ADR-0022: the downgrade is collective).
fn raw_assurance_with_quiet_anchor() -> raw::AssuranceEnvelope {
    let ledger = raw_source_named("ledger");
    let mut ledger_watermark = raw_watermark();
    ledger_watermark.source = Some(ledger.clone());
    let mut assurance = raw_assurance();
    assurance.required_sources.push(ledger.clone());
    assurance.observed_sources.push(ledger.clone());
    assurance.filter_epochs.push(raw::FilterEpochBySource {
        source: Some(ledger.clone()),
        filter_epoch: Some(9),
    });
    assurance.source_health.push(raw::SourceHealth {
        source: Some(ledger.clone()),
        healthy: true,
        detail: String::new(),
    });
    assurance.capabilities.push(raw::SourceCapabilityRecord {
        source: Some(ledger.clone()),
        capabilities: vec![raw::SourceCapability::CausalEdges as i32],
    });
    assurance.watermarks.push(ledger_watermark.clone());
    assurance.hot_settlement_watermarks.push(ledger_watermark);
    assurance
        .source_tiers_and_anchors
        .push(raw::SourceTierRecord {
            source: Some(ledger),
            tier: raw::SourceTier::Anchor as i32,
            anchored_event_ids: Vec::new(),
        });
    assurance
}

#[test]
fn quiet_co_required_anchor_keeps_an_authoritative_satisfy_at_the_public_boundary() {
    let envelope = raw_assurance_with_quiet_anchor();
    Verdict::try_from(raw_verdict(envelope.clone()))
        .expect("a quiet co-required anchor does not block an authoritative satisfy");

    let mut unanchored_envelope = envelope;
    unanchored_envelope.source_tiers_and_anchors[0]
        .anchored_event_ids
        .clear();
    assert_eq!(
        Verdict::try_from(raw_verdict(unanchored_envelope.clone())),
        Err(VerdictError::SourceTierGapMismatch)
    );

    unanchored_envelope.source_tier_gap = Some(raw::SourceTierGap {
        contributing_sources: Vec::new(),
        detail: "no required source anchored an Event".to_owned(),
        unanchored_sources: vec![raw_source(), raw_source_named("ledger")],
    });
    assert_eq!(
        Verdict::try_from(raw_verdict(unanchored_envelope.clone())),
        Err(VerdictError::MissingAnchorEvidence)
    );

    unanchored_envelope.cold_path = raw::TierSettlement::Pending as i32;
    unanchored_envelope.settlement_basis = None;
    let mut provisional = raw_verdict(unanchored_envelope);
    provisional.authority = raw::VerdictAuthority::Provisional as i32;
    Verdict::try_from(provisional)
        .expect("the declared source-tier gap keeps the wholly unanchored Verdict provisional");
}

#[test]
fn quiet_co_required_anchor_keeps_an_anchored_decisive_violation_authoritative() {
    let mut envelope = raw_assurance_with_quiet_anchor();
    envelope.settlement_basis = Some(raw::SettlementBasis {
        basis: Some(raw::settlement_basis::Basis::DecisiveViolation(
            raw::DecisiveViolationBasis {
                witness_events: vec![raw::DecisiveEvent {
                    event_id: "event".to_owned(),
                    source: Some(raw_source()),
                    sequence_epoch: Some(1),
                    sequence_number: Some(4),
                }],
                required_extraction_fact_ids: vec!["amount".to_owned()],
                invariance_proof_rule_id: "existence-witness".to_owned(),
                invariance_proof_rule_version: "v1".to_owned(),
                outcome_invariant_limitations: Some(raw::OutcomeInvariantLimitations {
                    gaps: Vec::new(),
                    unhealthy_sources: Vec::new(),
                    unresolved_causal_edges: Vec::new(),
                    unavailable_extraction_fact_ids: Vec::new(),
                    uncertainty: Vec::new(),
                }),
            },
        )),
    });
    let mut violation = raw_verdict(envelope);
    violation.outcome = raw::VerdictOutcome::Violate as i32;
    Verdict::try_from(violation)
        .expect("a quiet co-required anchor never un-settles an anchored decisive violation");
}
