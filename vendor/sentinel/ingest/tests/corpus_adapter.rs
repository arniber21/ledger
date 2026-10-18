//! Corpus Adapter tests over synthetic, in-memory `trace-corpus-v1` bundles.
//!
//! The #78 gate (`corpus_semantic_gate.rs`) exercises this Adapter against
//! vendored trace-grab output; these tests pin the Adapter's own contract:
//! the ADR-0015 split, the copied-causality edge rules, the opt-in
//! synchronous premise, delivery sequences, capability honesty, and the
//! ADR-0029 class boundary itself.

#![allow(clippy::doc_markdown)] // Product names read as prose here.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use sentinel_ingest::corpus::{Corpus, CorpusError};
use sentinel_ingest::corpus_adapter::{
    application_of, effective_capabilities, event_id, CorpusAdapter, SOURCE_ID,
};
use sentinel_ingest::evidence::ObservedEvidence;
use sentinel_model::{
    AttributeKey, AttributeValue, DataClass, Event, EventId, SourceCapability, SourceTier,
};

/// One JSONL line, as the schema's worked example shapes it.
fn record_line(
    id: &str,
    trace_id: &str,
    parent_id: Option<&str>,
    name: &str,
    service: Option<&str>,
    links: &[(&str, &str)],
) -> String {
    let parent = match parent_id {
        Some(parent) => format!(r#""{parent}""#),
        None => "null".to_owned(),
    };
    let mut attributes = String::from("{");
    if let Some(service) = service {
        let _ = write!(attributes, r#""service.name": "{service}""#);
    }
    attributes.push('}');
    let links = links
        .iter()
        .map(|(link_trace, link_span)| {
            format!(
                r#"{{"trace_id": "{link_trace}", "span_id": "{link_span}", "attributes": {{}}}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"id": "{id}", "trace_id": "{trace_id}", "parent_id": {parent}, "name": "{name}",
            "kind": "tool", "start": "2026-08-01T14:03:22.104Z", "end": "2026-08-01T14:03:23.000Z",
            "status": "ok", "error": null, "inputs": null, "outputs": null,
            "attributes": {attributes}, "labels": [], "links": [{links}], "unmapped": {{}},
            "source": {{"vendor": "otlp"}}}}"#
    )
}

fn corpus_of(lines: &[String]) -> Corpus {
    let text = lines
        .iter()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .and_then(|value| serde_json::to_string(&value))
                .expect("fixture is valid JSON")
        })
        .collect::<Vec<_>>()
        .join("\n");
    Corpus::from_jsonl(text.as_bytes()).expect("synthetic corpus decodes")
}

fn adapt(corpus: &Corpus) -> ObservedEvidence {
    CorpusAdapter::new().adapt(corpus).expect("corpus adapts")
}

fn predecessors(evidence: &ObservedEvidence) -> BTreeMap<EventId, BTreeSet<EventId>> {
    evidence
        .events()
        .iter()
        .map(|event| (event.id(), event.causal_predecessors()))
        .collect()
}

fn id_of(record_id: &str, phase: &str) -> EventId {
    EventId::new(event_id(record_id, phase)).expect("composed identity")
}

/// Two Events per record, with the ADR-0015 split's kinds and coordinates.
#[test]
fn every_record_becomes_two_events_with_split_kinds_and_sequences() {
    let corpus = corpus_of(&[
        record_line("a", "t1", None, "authenticate", Some("payments"), &[]),
        record_line("b", "t1", Some("a"), "charge", Some("payments"), &[]),
        record_line("c", "t1", None, "plan", None, &[]),
    ]);
    let evidence = adapt(&corpus);
    assert_eq!(evidence.events().len(), 6);

    let by_id: BTreeMap<EventId, &Event> = evidence
        .events()
        .iter()
        .map(|event| (event.id(), event))
        .collect();

    for (record_id, sequence) in [("a", 1_u64), ("b", 2_u64)] {
        for (phase, expected_sequence) in
            [("requested", sequence * 2 - 1), ("completed", sequence * 2)]
        {
            let event = by_id
                .get(&id_of(record_id, phase))
                .unwrap_or_else(|| panic!("{record_id}:{phase} exists"));
            assert_eq!(
                event.producer().kind().as_str(),
                format!("{}.{}", record_name(record_id), phase),
                "the kind is the record's name plus the phase"
            );
            assert_eq!(event.delivery().sequence().value(), expected_sequence);
            assert_eq!(event.delivery().epoch().value(), 0);
        }
    }

    // A record without service.name attributes is its vendor's application.
    let planned = by_id
        .get(&id_of("c", "requested"))
        .expect("vendor record exists");
    assert_eq!(
        planned.source().monitored_application().as_str(),
        "otlp",
        "no service.name means the parser identity is the application"
    );
    assert_eq!(planned.source().source().as_str(), SOURCE_ID);

    // The application half comes from service.name when present.
    let charged = by_id.get(&id_of("b", "requested")).expect("charge exists");
    assert_eq!(
        charged.source().monitored_application().as_str(),
        "payments"
    );
}

fn record_name(record_id: &str) -> &'static str {
    match record_id {
        "a" => "authenticate",
        "b" => "charge",
        "c" => "plan",
        other => panic!("unknown record {other}"),
    }
}

/// Edge classes 1, 2 and 4 are produced from structure alone; 3 only opt-in.
#[test]
fn copied_causality_produces_rules_one_two_and_four_and_never_three_by_default() {
    let corpus = corpus_of(&[
        record_line("root", "t1", None, "plan", Some("agent"), &[]),
        record_line("child", "t1", Some("root"), "tool", Some("agent"), &[]),
        record_line(
            "linked",
            "t1",
            None,
            "summarize",
            Some("agent"),
            &[("t1", "root")],
        ),
        // A parent no record carries: a dangling edge, preserved.
        record_line("orphan", "t1", Some("absent"), "retry", Some("agent"), &[]),
        // A link to a record no corpus carries: dangling too.
        record_line(
            "speculative",
            "t1",
            None,
            "guess",
            Some("agent"),
            &[("t9", "nowhere")],
        ),
    ]);

    let edges = predecessors(&adapt(&corpus));

    // Rule 1: requested before completed, for every record.
    for record_id in ["root", "child", "linked", "orphan", "speculative"] {
        assert!(
            edges[&id_of(record_id, "completed")].contains(&id_of(record_id, "requested")),
            "{record_id}: the split's own edge"
        );
    }

    // Rule 2: parent requested before child requested, dangling or not.
    assert!(edges[&id_of("child", "requested")].contains(&id_of("root", "requested")));
    assert!(edges[&id_of("orphan", "requested")].contains(&id_of("absent", "requested")));

    // Rule 3: absent by default — nesting alone does not order completions.
    assert!(!edges[&id_of("root", "completed")].contains(&id_of("child", "completed")));

    // Rule 4: a link reads as "target's span entirely precedes mine", so the
    // linked record's requested Event gains both of the target's phases and
    // its completed Event gains the target's completion. Dangling, preserved.
    assert!(edges[&id_of("linked", "requested")].contains(&id_of("root", "requested")));
    assert!(edges[&id_of("linked", "requested")].contains(&id_of("root", "completed")));
    assert!(edges[&id_of("linked", "completed")].contains(&id_of("root", "completed")));
    assert!(edges[&id_of("speculative", "requested")].contains(&id_of("nowhere", "requested")));
    assert!(edges[&id_of("speculative", "requested")].contains(&id_of("nowhere", "completed")));

    // Opt-in: the same corpus under the named premise gains rule 3, and only
    // onto the parent's completed Event — never a backwards edge on the child.
    let sync = CorpusAdapter::new()
        .with_synchronous_call_graph()
        .adapt(&corpus)
        .expect("corpus adapts under the named premise");
    let sync_edges = predecessors(&sync);
    assert!(sync_edges[&id_of("root", "completed")].contains(&id_of("child", "completed")));
    assert!(
        !sync_edges[&id_of("child", "requested")].contains(&id_of("child", "completed")),
        "rule 3 lands on the parent's completed Event, never backwards on the child"
    );
    assert!(CorpusAdapter::new()
        .with_synchronous_call_graph()
        .emits_synchronous_call_edges());
    assert!(!CorpusAdapter::new().emits_synchronous_call_edges());
}

/// A record with no recorded end emits only its requested Event, and the
/// even delivery slot it never filled is an honest gap.
#[test]
fn a_truncated_record_is_one_event_and_an_honest_sequence_gap() {
    let line = record_line("a", "t1", None, "authenticate", Some("payments"), &[])
        .replace(r#""end": "2026-08-01T14:03:23.000Z""#, r#""end": null"#);
    let corpus = corpus_of(&[line]);
    let evidence = adapt(&corpus);

    assert_eq!(
        evidence.events().len(),
        1,
        "only the requested Event exists"
    );
    assert_eq!(evidence.events()[0].id(), id_of("a", "requested"));
    assert_eq!(evidence.events()[0].delivery().sequence().value(), 1);

    let source = &evidence.sources().values().next().expect("one source");
    assert!(!source.is_contiguous(), "the unfilled even slot is a gap");
    assert_eq!(source.gaps().len(), 1);
    assert_eq!(source.gaps()[0].first_missing, 2);
    assert_eq!(source.gaps()[0].last_missing, 2);
}

/// Clocks are inert: capabilities withheld, uncertainty equal to the
/// timestamp's own precision, clock domains per application.
#[test]
fn clocks_are_inert_and_uncertainty_is_the_timestamps_own_precision() {
    let corpus = corpus_of(&[record_line(
        "a",
        "t1",
        None,
        "authenticate",
        Some("payments"),
        &[],
    )]);
    let evidence = adapt(&corpus);
    for event in evidence.events() {
        let capabilities = event.capabilities();
        assert!(capabilities.contains(SourceCapability::CausalEdges));
        assert!(capabilities.contains(SourceCapability::CompletenessSignals));
        assert!(!capabilities.contains(SourceCapability::BoundedClockUncertainty));
        assert!(!capabilities.contains(SourceCapability::ObserveBeforeEffect));
        // .104 is millisecond precision, so a millisecond uncertainty.
        assert_eq!(event.occurrence_time().uncertainty_nanoseconds(), 1_000_000);
        assert_eq!(
            event.occurrence_time().clock_domain().as_str(),
            "corpus:payments",
            "the clock domain is per application"
        );
    }
    let left = &evidence.events()[0];
    let right = &evidence.events()[1];
    assert!(matches!(
        sentinel_model::compare_occurrence_times(left, right),
        sentinel_model::ClockRelation::MissingBoundedClockUncertainty { .. }
    ));
    assert_eq!(effective_capabilities().iter().count(), 2);
}

/// Every corpus Event is Corpus-class, and the class never leaks between
/// Adapters: the ADR-0029 boundary as a testable invariant.
#[test]
fn every_corpus_event_is_corpus_class_and_never_tenant_class() {
    let corpus = corpus_of(&[record_line(
        "a",
        "t1",
        None,
        "authenticate",
        Some("payments"),
        &[],
    )]);
    let evidence = adapt(&corpus);
    assert!(!evidence.events().is_empty());
    for event in evidence.events() {
        assert_eq!(event.data_class(), DataClass::Corpus);
    }

    // The other Adapter stays Tenant-class: the two paths do not mix, and the
    // boundary is visible from either side.
    let capture = sentinel_ingest::capture::Capture::from_json(
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("testdata")
                .join("sagashop")
                .join("ts-happy.json"),
        )
        .expect("vendored capture is readable")
        .as_slice(),
    )
    .expect("vendored capture decodes");
    let tenant = sentinel_ingest::sagashop::SagaShopAdapter::with_synchronous_call_graph()
        .adapt(&capture)
        .expect("capture adapts");
    for event in tenant.events() {
        assert_eq!(event.data_class(), DataClass::Tenant);
    }
}

/// Tiers default to Contributing and can be stated per application.
#[test]
fn tiers_default_to_contributing_and_can_be_stated() {
    let corpus = corpus_of(&[
        record_line("a", "t1", None, "authenticate", Some("payments"), &[]),
        record_line("b", "t1", None, "plan", None, &[]),
    ]);

    for event in adapt(&corpus).events() {
        assert_eq!(event.source_tier(), SourceTier::Contributing);
    }

    let stated = CorpusAdapter::new()
        .with_source_tiers([("payments".to_owned(), SourceTier::Anchor)])
        .adapt(&corpus)
        .expect("corpus adapts");
    for event in stated.events() {
        let expected = match event.source().monitored_application().as_str() {
            "payments" => SourceTier::Anchor,
            _ => SourceTier::Contributing,
        };
        assert_eq!(event.source_tier(), expected);
    }
}

/// Duplicate record ids are rejected with the second record's line number.
#[test]
fn duplicate_record_ids_are_rejected_by_line() {
    let corpus = corpus_of(&[
        record_line("a", "t1", None, "authenticate", Some("payments"), &[]),
        record_line("a", "t2", None, "authenticate", Some("payments"), &[]),
    ]);
    assert_eq!(
        CorpusAdapter::new().adapt(&corpus),
        Err(
            sentinel_ingest::corpus_adapter::AdapterError::DuplicateRecordId {
                id: "a".to_owned(),
                line: 2,
            }
        )
    );
}

/// Structural failures are precise errors, never panics, at every level.
#[test]
fn malformed_corpora_fail_structurally_never_panic() {
    let cases: &[(&str, &str)] = &[
        ("not json at all", "Malformed"),
        ("{}", "MissingField"),
        (
            r#"{"id":"a","trace_id":"t","parent_id":null,"name":"n","kind":"k",
                "start":"2026-08-01T14:03:22.104Z","end":null,"status":"weird","error":null,
                "inputs":null,"outputs":null,"attributes":{},"labels":[],"links":[],"unmapped":{},
                "source":{"vendor":"v"}}"#,
            "InvalidField",
        ),
        (
            r#"{"id":"a","trace_id":"t","parent_id":null,"name":"n","kind":"k",
                "start":"2026-08-01T14:03:22.104Z","end":null,"status":"ok","error":null,
                "inputs":null,"outputs":null,"attributes":{},"labels":[],"links":[],"unmapped":{},
                "source":{"vendor":"v"},"extra":1}"#,
            "UnknownField",
        ),
        (
            r#"{"id":"a","trace_id":"t","parent_id":null,"name":"n","kind":"k",
                "start":"not-a-timestamp","end":null,"status":"ok","error":null,
                "inputs":null,"outputs":null,"attributes":{},"labels":[],"links":[],"unmapped":{},
                "source":{"vendor":"v"}}"#,
            "InvalidField",
        ),
    ];
    for (text, variant) in cases {
        let compact = serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|value| serde_json::to_string(&value).ok())
            .unwrap_or_else(|| (*text).to_owned());
        let error = Corpus::from_jsonl(compact.as_bytes())
            .expect_err("structurally invalid corpora are rejected");
        let rendered = error.to_string();
        assert!(
            rendered.contains(variant) || format!("{error:?}").contains(variant),
            "{text}: error {error:?} should be {variant}"
        );
    }
    assert_eq!(Corpus::from_jsonl(b"\n"), Err(CorpusError::Empty));
}

/// The record's own attributes are carried under their own keys, and the
/// adapter's structural context lives under `corpus.*`.
#[test]
fn record_attributes_and_structural_context_coexist() {
    let line = record_line("a", "t1", None, "authenticate", Some("payments"), &[]).replace(
        r#""attributes": {"service.name": "payments"}"#,
        r#""attributes": {"service.name": "payments", "order.id": "TOK_1", "amount": 42}"#,
    );
    let evidence = adapt(&corpus_of(&[line]));
    let event = &evidence.events()[0];
    let attributes = event.producer().attributes();

    assert_eq!(
        attributes.get(&AttributeKey::new("order.id").expect("fixture")),
        Some(&AttributeValue::String("TOK_1".to_owned())),
        "the record's own attributes are carried under their own keys"
    );
    assert_eq!(
        attributes.get(&AttributeKey::new("amount").expect("fixture")),
        Some(&AttributeValue::Integer(42)),
        "integral numbers stay integers"
    );
    for key in [
        "corpus.record.id",
        "corpus.trace.id",
        "corpus.name",
        "corpus.kind",
        "corpus.status",
        "corpus.source.vendor",
        "corpus.record.line",
        "corpus.end.recorded",
    ] {
        assert!(
            attributes.contains_key(&AttributeKey::new(key).expect("fixture")),
            "{key} carries the record's structure"
        );
    }
    assert_eq!(
        attributes.get(&AttributeKey::new("corpus.record.id").expect("fixture")),
        Some(&AttributeValue::String("a".to_owned()))
    );
}

/// The application half is the service.name convention, else the vendor.
#[test]
fn application_of_follows_the_service_name_convention() {
    let with_service = corpus_of(&[record_line("a", "t1", None, "n", Some("shipping"), &[])]);
    assert_eq!(application_of(&with_service.records[0]), "shipping");

    let without = corpus_of(&[record_line("b", "t1", None, "n", None, &[])]);
    assert_eq!(application_of(&without.records[0]), "otlp");
}
