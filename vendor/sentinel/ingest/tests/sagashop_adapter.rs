//! Adapter unit tests against the vendored, unmodified SagaShop captures.
//!
//! Everything here is checked against real artifacts rather than asserted in
//! prose, including the two premises the Adapter's soundness rests on: the
//! transition-uniqueness rule ported from `shared/monitor.ts`, and the
//! synchronous-call-graph claim behind edge class 3.

#![allow(clippy::doc_markdown)] // Product names read as prose here.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sentinel_ingest::capture::{Capture, CaptureAttribute, CaptureError, CapturedSpan, SpanKind};
use sentinel_ingest::evidence::ObservedEvidence;
use sentinel_ingest::sagashop::{
    effective_capabilities, event_id, source_tier, transition, SagaShopAdapter,
    OCCURRENCE_UNCERTAINTY_NANOSECONDS, PHASE_COMPLETED, PHASE_REQUESTED,
};
use sentinel_model::{Event, EventId, SourceCapability, SourceTier};

fn fixture_path(scenario: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("testdata")
        .join("sagashop")
        .join(format!("ts-{scenario}.json"))
}

fn load(scenario: &str) -> Capture {
    let bytes = std::fs::read(fixture_path(scenario)).expect("vendored capture is readable");
    Capture::from_json(&bytes).expect("vendored capture decodes")
}

fn adapt(capture: &Capture) -> ObservedEvidence {
    SagaShopAdapter::with_synchronous_call_graph()
        .adapt(capture)
        .expect("capture adapts")
}

fn predecessors(evidence: &ObservedEvidence) -> BTreeMap<EventId, BTreeSet<EventId>> {
    evidence
        .events()
        .iter()
        .map(|event| (event.id(), event.causal_predecessors()))
        .collect()
}

fn id(span: &CapturedSpan, phase: &str) -> EventId {
    EventId::new(event_id(
        &span.logical_trace_id,
        &span.logical_span_id,
        phase,
    ))
    .expect("composed identity")
}

fn span_with_event<'a>(capture: &'a Capture, event: &str) -> &'a CapturedSpan {
    capture
        .spans
        .iter()
        .find(|span| span.sagashop_text("sagashop.event") == Some(event))
        .unwrap_or_else(|| panic!("capture carries a {event} span"))
}

/// Two Events per span, with the specified kinds and delivery coordinates.
#[test]
fn every_span_becomes_exactly_two_events_with_the_specified_coordinates() {
    for scenario in ["happy", "cancel", "hazards"] {
        let capture = load(scenario);
        let evidence = adapt(&capture);
        assert_eq!(
            evidence.events().len(),
            capture.spans.len() * 2,
            "{scenario}: a Span is a derived view over a matched pair of Events"
        );

        let by_id: BTreeMap<EventId, &Event> = evidence
            .events()
            .iter()
            .map(|event| (event.id(), event))
            .collect();

        for span in &capture.spans {
            for (phase, sequence) in [
                (PHASE_REQUESTED, span.source_sequence * 2 - 1),
                (PHASE_COMPLETED, span.source_sequence * 2),
            ] {
                let event = by_id
                    .get(&id(span, phase))
                    .unwrap_or_else(|| panic!("{scenario}: {phase} Event exists"));
                let expected_kind = span.sagashop_text("sagashop.event").map_or_else(
                    || format!("otel.span.{phase}"),
                    |event| format!("sagashop.{event}.{phase}"),
                );
                assert_eq!(event.producer().kind().as_str(), expected_kind);
                assert_eq!(event.delivery().sequence().value(), sequence);
                assert_eq!(event.delivery().epoch().value(), 0);
                assert_eq!(
                    event.source().monitored_application().as_str(),
                    span.service,
                    "the source key discriminates on the service name"
                );
            }
        }
    }
}

/// Edge classes 1, 2 and 4 are produced, and 3 only from the named constructor.
#[test]
fn every_declared_edge_class_is_produced() {
    let capture = load("happy");
    let evidence = adapt(&capture);
    let edges = predecessors(&evidence);
    let by_logical: BTreeMap<&str, &CapturedSpan> = capture
        .spans
        .iter()
        .map(|span| (span.logical_span_id.as_str(), span))
        .collect();

    let mut saw_parent_edge = false;
    let mut saw_child_edge = false;
    for span in &capture.spans {
        // Class 1: a span is requested before it completes.
        assert!(edges[&id(span, PHASE_COMPLETED)].contains(&id(span, PHASE_REQUESTED)));

        if let Some(parent) = span
            .logical_parent_span_id
            .as_deref()
            .and_then(|parent| by_logical.get(parent))
        {
            // Class 2: a parent is requested before its child is requested.
            assert!(edges[&id(span, PHASE_REQUESTED)].contains(&id(parent, PHASE_REQUESTED)));
            saw_parent_edge = true;
            // Class 3: a child completes before its parent completes.
            assert!(edges[&id(parent, PHASE_COMPLETED)].contains(&id(span, PHASE_COMPLETED)));
            saw_child_edge = true;
        }
    }
    assert!(saw_parent_edge && saw_child_edge);

    // Class 4: the order-state transition link. The happy capture's chain runs
    // create -> charge -> fulfill -> ship -> deliver, and this is the edge that
    // carries the charge-to-ship proof.
    let charge = span_with_event(&capture, "order.charge");
    let fulfill = span_with_event(&capture, "order.fulfill");
    let ship = span_with_event(&capture, "order.ship");
    assert!(edges[&id(fulfill, PHASE_REQUESTED)].contains(&id(charge, PHASE_COMPLETED)));
    assert!(edges[&id(ship, PHASE_REQUESTED)].contains(&id(fulfill, PHASE_COMPLETED)));

    // Class 3 is not a default. Without the named constructor it is absent, and
    // the payment Event then has no outgoing edge at all.
    let withheld = SagaShopAdapter::without_synchronous_call_graph()
        .adapt(&capture)
        .expect("capture adapts without the synchronous premise");
    assert!(!SagaShopAdapter::without_synchronous_call_graph().emits_synchronous_call_edges());
    let withheld_edges = predecessors(&withheld);
    for span in &capture.spans {
        if let Some(parent) = span
            .logical_parent_span_id
            .as_deref()
            .and_then(|parent| by_logical.get(parent))
        {
            assert!(
                !withheld_edges[&id(parent, PHASE_COMPLETED)].contains(&id(span, PHASE_COMPLETED))
            );
        }
    }
}

/// Edge class 3's premise is checked against the artifact, not asserted in prose.
///
/// # Fixture-truth finding
///
/// The premise cannot be checked against `endMs`, and this test records why
/// rather than degenerating into a vacuous comparison. `durMs == 0` and
/// `startMs == endMs` on every span in every capture: these artifacts contain
/// no span *durations* at all, only a synthetic per-operation step counter,
/// and the `ts` port stamps that counter **only on SERVER spans** — CLIENT
/// and INTERNAL spans carry `startMs = endMs = 0`. Every parent/child pair
/// therefore has at least one unstamped endpoint, so no pair is comparable at
/// all. Worse, the stamps are not nesting consistent even between SERVER
/// spans — `sagashop-payment`'s `POST /charge` ends at `21000` inside
/// `sagashop-order`'s `POST /orders/.../charge`, which ends at `20000`. Wall
/// time in these artifacts is diagnostics only, exactly as SagaShop's README
/// says, which is also why `BoundedClockUncertainty` is withheld.
///
/// What *is* checkable, and is checked here, is that the capture uses only
/// RPC-shaped CLIENT/SERVER nesting with no non-RPC topology: a SERVER
/// span's logical parent is always a CLIENT span in the calling service, and
/// a CLIENT span's parent is always a SERVER or INTERNAL span in its own
/// service. That is span-kind hygiene, not a synchrony check — a
/// fire-and-forget call produces an identical shape, since OTel context
/// propagation parents spans the same way whether or not the caller awaits.
/// It cannot license "the child completes before the parent completes" on
/// its own. The await premise itself remains licensed by `contract.yaml`'s
/// synchronous route contract (see
/// `SagaShopAdapter::with_synchronous_call_graph`), not by anything
/// checkable in these artifacts.
#[test]
fn edge_class_three_premise_holds_structurally_and_not_on_the_wall_clock() {
    for scenario in ["happy", "cancel", "hazards"] {
        let capture = load(scenario);
        let by_logical: BTreeMap<&str, &CapturedSpan> = capture
            .spans
            .iter()
            .map(|span| (span.logical_span_id.as_str(), span))
            .collect();

        let mut checked = 0_usize;
        let mut wall_clock_comparable = 0_usize;
        for span in &capture.spans {
            let Some(parent) = span
                .logical_parent_span_id
                .as_deref()
                .and_then(|parent| by_logical.get(parent))
            else {
                continue;
            };
            checked += 1;
            match (span.kind, parent.kind) {
                (SpanKind::Server, SpanKind::Client) => assert_ne!(
                    span.service, parent.service,
                    "{scenario}: a remote handler runs in a different service from its caller"
                ),
                (SpanKind::Client, SpanKind::Server | SpanKind::Internal) => assert_eq!(
                    span.service, parent.service,
                    "{scenario}: an outbound call is issued by the span that awaits it"
                ),
                other => panic!("{scenario}: unexpected parent/child span kinds {other:?}"),
            }
            if span.end_ms > 0 && parent.end_ms > 0 {
                wall_clock_comparable += 1;
            }
        }

        assert!(checked > 0, "{scenario}: there are edge-3 pairs to check");
        assert_eq!(
            wall_clock_comparable, 0,
            "{scenario}: if a pair ever becomes wall-clock comparable, revisit this test's premise"
        );
    }

    // The concrete counterexample, pinned so a future capture that fixes the
    // clock fails here loudly instead of leaving this reasoning stale.
    let happy = load("happy");
    let payment = span_with_event(&happy, "payment.charge");
    let order_charge = span_with_event(&happy, "order.charge");
    assert!(
        payment.end_ms > order_charge.end_ms,
        "the callee's wall clock runs past its caller's, so wall time cannot order these"
    );
}

/// Rule 4 is a real port: the full transition triple, self excluded, unique only.
#[test]
fn rule_four_requires_the_full_transition_triple_and_excludes_self() {
    let capture = load("happy");
    let charge = span_with_event(&capture, "order.charge");
    assert_eq!(
        transition(charge),
        Some(("779d1470-d075-4e60-ada4-3c2a72064df1", "RESERVED", "PAID"))
    );

    // A span carrying `state` without `prev_state` is not a transition at all.
    // Under a looser predicate it would inflate the candidate set for `PAID`,
    // uniqueness would fail, no edge would be emitted into `order.fulfill`, the
    // charge-to-ship chain would break, and the happy path would return
    // Inconclusive instead of Satisfy.
    let mut synthetic = capture.clone();
    let position = synthetic
        .spans
        .iter()
        .position(|span| span.sagashop_text("sagashop.event") == Some("order.charge"))
        .expect("the happy capture charges");
    let mut copy = synthetic.spans[position].clone();
    copy.logical_span_id = "00ffee0011223344".to_owned();
    copy.span_id = copy.logical_span_id.clone();
    copy.source_sequence = synthetic
        .spans
        .iter()
        .filter(|span| span.service == copy.service)
        .map(|span| span.source_sequence)
        .max()
        .expect("service has spans")
        + 1;
    copy.attributes.remove("sagashop.order.prev_state");
    assert_eq!(
        transition(&copy),
        None,
        "state without prev_state is not a transition"
    );
    synthetic.spans.push(copy);

    let fulfill = span_with_event(&synthetic, "order.fulfill");
    let edges = predecessors(&adapt(&synthetic));
    assert!(
        edges[&id(fulfill, PHASE_REQUESTED)].contains(&id(charge, PHASE_COMPLETED)),
        "the strict predicate keeps the original charge the unique PAID predecessor"
    );

    // Now duplicate the charge span *with* its full triple, reproducing hazard
    // H1's signature of two spans writing PAID for one order. Uniqueness fails
    // and rule 4 emits nothing, so the racing Events stay Concurrent instead of
    // Sentinel picking a schedule: the unsoundness and the missing edge appear
    // together.
    let mut racing = capture.clone();
    let mut copy = racing.spans[position].clone();
    copy.logical_span_id = "00ffee0055667788".to_owned();
    copy.span_id = copy.logical_span_id.clone();
    copy.source_sequence = racing
        .spans
        .iter()
        .filter(|span| span.service == copy.service)
        .map(|span| span.source_sequence)
        .max()
        .expect("service has spans")
        + 1;
    racing.spans.push(copy);

    let edges = predecessors(&adapt(&racing));
    assert!(
        !edges[&id(fulfill, PHASE_REQUESTED)].contains(&id(charge, PHASE_COMPLETED)),
        "two spans writing PAID make the predecessor non-unique, so no edge is emitted"
    );
}

/// The hazards capture really does carry H1's duplicate PAID write, and never ships that order.
#[test]
fn the_hazard_capture_carries_the_double_charge_signature() {
    let capture = load("hazards");
    let mut paid_writers: BTreeMap<&str, usize> = BTreeMap::new();
    let mut shipped: BTreeSet<&str> = BTreeSet::new();
    for span in &capture.spans {
        let Some(order) = span.sagashop_text("sagashop.order.id") else {
            continue;
        };
        if span.sagashop_text("sagashop.order.state") == Some("PAID") {
            *paid_writers.entry(order).or_default() += 1;
        }
        if span.sagashop_text("sagashop.event") == Some("order.ship") {
            shipped.insert(order);
        }
    }
    let doubly_charged: Vec<&str> = paid_writers
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(order, _)| *order)
        .collect();
    assert_eq!(
        doubly_charged.len(),
        1,
        "H1 charges exactly one order twice"
    );
    assert!(
        !shipped.contains(doubly_charged[0]),
        "the double-charged order never ships, so it creates no P4 Binding"
    );
    assert_eq!(shipped.len(), 1, "only the H2 order ships");
}

/// The withheld capabilities and the recorded uncertainty are pinned together.
///
/// They are asserted in one test on purpose: `1_000_000` is inert only because
/// the capability is withheld, so the pair must not be able to drift apart. The
/// day someone grants `BoundedClockUncertainty` to this source, `0` would make
/// `compare_occurrence_intervals` confidently return `Before` for two
/// 1 ms-resolution timestamps 1 ms apart, where `1_000_000` correctly returns
/// `Overlapping`. `0` is a false claim that merely happens to be unread.
#[test]
fn clocks_are_inert_and_the_recorded_uncertainty_says_why() {
    let capture = load("happy");
    let evidence = adapt(&capture);

    for event in evidence.events() {
        let capabilities = event.capabilities();
        assert!(capabilities.contains(SourceCapability::CausalEdges));
        assert!(capabilities.contains(SourceCapability::CompletenessSignals));
        assert!(!capabilities.contains(SourceCapability::BoundedClockUncertainty));
        assert!(!capabilities.contains(SourceCapability::ObserveBeforeEffect));
        assert_eq!(
            event.occurrence_time().uncertainty_nanoseconds(),
            OCCURRENCE_UNCERTAINTY_NANOSECONDS
        );
        assert_eq!(
            event.occurrence_time().clock_domain().as_str(),
            format!(
                "sagashop:{}:{}",
                capture.port,
                event.source().monitored_application()
            ),
            "the clock domain is per service, so cross-service pairs are never comparable"
        );
    }

    // Withholding the capability makes the model refuse to order by clock, which
    // is ADR-0018 enforced by construction rather than by convention.
    let left = &evidence.events()[0];
    let right = &evidence.events()[2];
    assert!(matches!(
        sentinel_model::compare_occurrence_times(left, right),
        sentinel_model::ClockRelation::MissingBoundedClockUncertainty { .. }
    ));

    assert_eq!(effective_capabilities().iter().count(), 2);
}

/// Anchor tier is assigned to the systems the order service calls through.
#[test]
fn source_tiers_follow_the_assurance_argument() {
    assert_eq!(source_tier("sagashop-payment"), SourceTier::Anchor);
    assert_eq!(source_tier("sagashop-inventory"), SourceTier::Anchor);
    assert_eq!(source_tier("sagashop-shipping"), SourceTier::Anchor);
    assert_eq!(source_tier("sagashop-order"), SourceTier::Contributing);
    assert_eq!(source_tier("sagashop-driver"), SourceTier::Contributing);
    // An unrecognised source has no argument entitling it to anchor status.
    assert_eq!(source_tier("sagashop-unknown"), SourceTier::Contributing);

    let evidence = adapt(&load("happy"));
    for (source, record) in evidence.sources() {
        assert_eq!(
            record.tier(),
            source_tier(source.monitored_application().as_str())
        );
        assert!(record.is_contiguous(), "the vendored capture is contiguous");
        assert_eq!(
            record.completeness().complete_through(),
            record.completeness().last_emitted(),
            "a closed recording is complete through everything it emitted"
        );
    }
}

/// Version stamps and unknown members are rejected at the untrusted boundary.
#[test]
fn version_mismatches_and_unknown_members_are_rejected() {
    let text = std::fs::read_to_string(fixture_path("happy")).expect("fixture is readable");

    let wrong_artifact = text.replace("capture-envelope-v1", "capture-envelope-v2");
    assert!(matches!(
        Capture::from_json(wrong_artifact.as_bytes()),
        Err(CaptureError::UnsupportedVersion {
            field: "artifactVersion",
            ..
        })
    ));

    let wrong_contract = text.replace("sagashop-contract-v1", "sagashop-contract-v2");
    assert!(matches!(
        Capture::from_json(wrong_contract.as_bytes()),
        Err(CaptureError::UnsupportedVersion {
            field: "contractVersion",
            ..
        })
    ));

    let extra_member = text.replacen("\"port\":", "\"future\": 1,\n  \"port\":", 1);
    assert!(matches!(
        Capture::from_json(extra_member.as_bytes()),
        Err(CaptureError::UnknownField { .. })
    ));
}

/// An unresolved logical parent is a dangling causal edge, never a parse error.
///
/// `shared/capture.ts` rejects it; this decoder deliberately does not. ADR-0019
/// names missing parents as a case the tracer bullet must handle as *evidence*,
/// and rejecting it here would make the Core's fourth evidence-gate disjunct
/// untestable.
#[test]
fn an_unresolved_logical_parent_becomes_a_dangling_edge() {
    let mut capture = load("happy");
    let position = capture
        .spans
        .iter()
        .position(|span| span.sagashop_text("sagashop.event") == Some("shipping.schedule"))
        .expect("the happy capture schedules a shipment");
    capture.spans[position].logical_parent_span_id = Some("0000000000000000".to_owned());
    let absent = EventId::new(event_id(
        &capture.spans[position].logical_trace_id,
        "0000000000000000",
        PHASE_REQUESTED,
    ))
    .expect("composed identity");
    let span = capture.spans[position].clone();

    let evidence = adapt(&capture);
    let edges = predecessors(&evidence);
    assert!(
        edges[&id(&span, PHASE_REQUESTED)].contains(&absent),
        "the unresolved parent is emitted as a predecessor no Event satisfies"
    );
    assert!(
        !evidence.events().iter().any(|event| event.id() == absent),
        "and nothing in the capture satisfies it"
    );
}

/// A reused `sourceSequence` never reaches the Core.
#[test]
fn a_reused_source_sequence_is_rejected_by_the_adapter() {
    let mut capture = load("happy");
    let position = capture
        .spans
        .iter()
        .position(|span| span.sagashop_text("sagashop.event") == Some("order.ship"))
        .expect("the happy capture ships");
    capture.spans[position].source_sequence = 1;
    assert!(SagaShopAdapter::with_synchronous_call_graph()
        .adapt(&capture)
        .is_err());
}

/// Attributes are carried verbatim, with the OpenTelemetry context alongside.
#[test]
fn adapted_attributes_carry_the_sagashop_evidence_and_the_span_context() {
    let capture = load("happy");
    let evidence = adapt(&capture);
    let charge = span_with_event(&capture, "payment.charge");
    let event = evidence
        .events()
        .iter()
        .find(|event| event.id() == id(charge, PHASE_COMPLETED))
        .expect("the charge completed Event exists");

    let attributes = event.producer().attributes();
    let mut carried = 0_usize;
    for (key, value) in &charge.attributes {
        if !key.starts_with("sagashop.") {
            continue;
        }
        if let CaptureAttribute::String(text) = value {
            carried += 1;
            assert_eq!(
                attributes.get(&sentinel_model::AttributeKey::new(key.clone()).expect("fixture")),
                Some(&sentinel_model::AttributeValue::String(text.clone())),
                "sagashop.* attributes are carried verbatim"
            );
        }
    }
    assert!(carried > 0, "the charge span carries sagashop evidence");
    for key in [
        "otel.span.id",
        "otel.trace.id",
        "otel.parent_span.id",
        "otel.span.name",
        "otel.span.kind",
        "otel.span.status",
        "otel.service.name",
        "otel.span.end_unix_nanos",
    ] {
        assert!(
            attributes.contains_key(&sentinel_model::AttributeKey::new(key).expect("fixture key")),
            "{key} accompanies the Event"
        );
    }
    assert!(
        !attributes
            .keys()
            .any(|key| key.as_str().starts_with("http.")),
        "only sagashop.* evidence and the span context are carried"
    );
}
