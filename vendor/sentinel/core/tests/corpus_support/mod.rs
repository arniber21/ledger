//! #78's converter: `capture-envelope-v1` → generic `trace-corpus-v1` input.
//!
//! The conversion lives **here**, in the sentinel repo, because trace-grab is a
//! public tool that must never learn an internal testbed format (#78's explicit
//! non-goal; trace-grab ADR-0013's "explicit non-decision"). It is a
//! test-support fixture-builder, not a production code path: its only consumer
//! is the semantic gate (`core/tests/corpus_semantic_gate.rs`) and the regeneration
//! recipe.
//!
//! # What is copied, and what is *authored*
//!
//! trace-grab ADR-0010 is normative: the corpus's causal content is `parent_id`
//! plus `links[]`, copied, never derived. The capture, however, carries causal
//! structure *beyond* its parent pointers — edge class 3 (the synchronous-call
//! completion chain) and edge class 4 (the order-state transition link), both
//! derived by `sagashop.rs` from the SagaShop route contract and
//! `shared/monitor.ts`. Encoding those edges as corpus `links[]` is the honest
//! division of labor: the conversion is SagaShop-aware by definition, so
//! SagaShop's derived causality enters the corpus *here*, as copied links,
//! and the corpus adapter still derives nothing. [`transition`] is re-exported
//! from `sagashop` rather than re-ported — a second copy of `monitor.ts`'s
//! rule would drift.
//!
//! # Field mapping
//!
//! | capture | corpus | note |
//! |---|---|---|
//! | `logicalSpanId` | `id` | the logical identity, not the raw OTel id |
//! | `logicalTraceId` | `trace_id` | ditto |
//! | `logicalParentSpanId` | `parent_id` | `null` for roots, dangling preserved |
//! | `sagashop.event` else `name` | `name` | the analysis vocabulary stays readable |
//! | `kind` | `kind` | verbatim |
//! | `startMs`/`endMs` | `start`/`end` | ISO-8601 UTC, millisecond precision |
//! | `status` | `status` | both are OTLP's three-value model |
//! | — | `attributes` | span attributes plus `service.name` |
//! | — | `links` | the derived edges (class 4), as links |
//! | — | `inputs`/`outputs`/`labels`/`unmapped` | empty/null: the capture has none |
//!
//! `service.name` rides in `attributes` because that is the OTel resource
//! convention the corpus adapter reads for its application half — the corpus
//! record shape has no dedicated service field, by design (trace-grab
//! ADR-0010: span-shaped, vendor-neutral).

#![allow(clippy::doc_markdown)] // Product names read as prose here.

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use sentinel_ingest::capture::{Capture, CaptureAttribute};
use sentinel_ingest::sagashop;

/// One conversion failure, naming the cause.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConversionError {
    /// A span attribute failed to map onto the corpus value model.
    Attribute(String),
    /// A millisecond stamp fell outside the representable date range.
    Timestamp(String),
}

impl std::fmt::Display for ConversionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Attribute(detail) => write!(formatter, "unmappable span attribute: {detail}"),
            Self::Timestamp(detail) => write!(formatter, "unmappable timestamp: {detail}"),
        }
    }
}

impl std::error::Error for ConversionError {}

/// The epoch the capture's millisecond stamps count from: the unix epoch.
const UNIX_EPOCH_MILLISECONDS: i64 = 0;

/// Converts milliseconds since the unix epoch to an ISO-8601 UTC timestamp
/// string at millisecond precision — the corpus's `start`/`end` shape, and the
/// precision the corpus adapter later reports as the clock's uncertainty.
fn iso8601_utc(milliseconds: i64) -> Result<String, ConversionError> {
    let seconds = milliseconds.div_euclid(1_000);
    let millis = milliseconds.rem_euclid(1_000);
    let days = seconds.div_euclid(86_400);
    let time_of_day = seconds.rem_euclid(86_400);

    // Civil-from-days, the inverse of the corpus decoder's days_from_civil.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };

    if !(0..=9_999).contains(&year) {
        return Err(ConversionError::Timestamp(format!(
            "year {year} is outside the four-digit RFC-3339 range"
        )));
    }
    let hour = time_of_day / 3_600;
    let minute = (time_of_day % 3_600) / 60;
    let second = time_of_day % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z"
    ))
}

/// Maps one capture attribute onto the corpus value model.
fn corpus_value(value: &CaptureAttribute) -> Result<Value, ConversionError> {
    match value {
        CaptureAttribute::Null => Ok(Value::Null),
        CaptureAttribute::Bool(value) => Ok(Value::Bool(*value)),
        CaptureAttribute::String(value) => Ok(Value::String(value.clone())),
        CaptureAttribute::Number(value) => serde_json::Number::from_f64(*value)
            .map(Value::Number)
            .ok_or_else(|| ConversionError::Attribute(format!("non-finite number {value}"))),
        CaptureAttribute::Array(values) => values
            .iter()
            .map(corpus_value)
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        CaptureAttribute::Object(entries) => {
            let mut object = Map::new();
            for (key, value) in entries {
                object.insert(key.clone(), corpus_value(value)?);
            }
            Ok(Value::Object(object))
        }
        // `CaptureAttribute` is non_exhaustive: an unknown future variant has
        // no corpus mapping, and refusing it is correct — the converter must
        // be updated knowingly when the capture schema grows.
        other => Err(ConversionError::Attribute(format!(
            "unmapped capture attribute variant {other:?}"
        ))),
    }
}

/// The capture's derived causal edges, expressed as corpus links.
///
/// Only edge class 4 — the order-state transition link, [`sagashop::transition`]'s
/// port of `monitor.ts` — is encoded, and only the *emitted* edge is: for a
/// successor span `s` with predecessor `p`, one link `{span_id: p}` on `s`.
/// Edge class 3 is not encoded because the corpus adapter's opt-in
/// `with_synchronous_call_graph()` reconstructs it from `parent_id` under the
/// same route-contract premise the capture adapter uses it under.
fn transition_links(capture: &Capture) -> BTreeMap<String, Vec<Value>> {
    let mut links: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for successor in &capture.spans {
        let Some((order, previous, _)) = sagashop::transition(successor) else {
            continue;
        };
        let mut candidates = capture.spans.iter().filter(|candidate| {
            sagashop::transition(candidate).is_some_and(|(candidate_order, _, candidate_state)| {
                candidate_order == order
                    && candidate_state == previous
                    && candidate.logical_span_id != successor.logical_span_id
            })
        });
        let (Some(predecessor), None) = (candidates.next(), candidates.next()) else {
            continue;
        };
        links
            .entry(successor.logical_span_id.clone())
            .or_default()
            .push(json!({
                "trace_id": successor.logical_trace_id,
                "span_id": predecessor.logical_span_id,
                "attributes": { "sagashop.transition": true },
            }));
    }
    links
}

/// Converts one structurally valid capture into generic corpus input records.
///
/// The output is the *pre-sanitization* corpus: vendor strings intact. Running
/// trace-grab over it produces the sanitized corpus; the semantic gate feeds
/// both through the ingest pipeline and asserts the property matrix survives.
///
/// # Errors
///
/// Returns [`ConversionError`] when an attribute or timestamp cannot be mapped
/// onto the corpus model, which for a vendored capture means the fixture no
/// longer matches its recorded provenance.
pub fn capture_to_corpus_records(capture: &Capture) -> Result<Vec<Value>, ConversionError> {
    let links_by_span = transition_links(capture);
    let mut records = Vec::with_capacity(capture.spans.len());
    for span in &capture.spans {
        let mut attributes = Map::new();
        attributes.insert(
            "service.name".to_owned(),
            Value::String(span.service.clone()),
        );
        for (key, value) in &span.attributes {
            attributes.insert(key.clone(), corpus_value(value)?);
        }

        records.push(json!({
            "id": span.logical_span_id,
            "trace_id": span.logical_trace_id,
            "parent_id": span.logical_parent_span_id,
            "name": span
                .sagashop_text("sagashop.event")
                .unwrap_or(&span.name),
            "kind": format!("{:?}", span.kind).to_uppercase(),
            "start": iso8601_utc(UNIX_EPOCH_MILLISECONDS + span.start_ms)?,
            "end": iso8601_utc(UNIX_EPOCH_MILLISECONDS + span.end_ms)?,
            "status": span.status,
            "error": null,
            "inputs": null,
            "outputs": null,
            "attributes": Value::Object(attributes),
            "labels": [],
            "links": links_by_span.get(&span.logical_span_id).cloned().unwrap_or_default(),
            "unmapped": {},
            "source": { "vendor": format!("sagashop-{}", capture.port) },
        }));
    }
    Ok(records)
}

/// Serializes corpus records as the JSONL the trace-grab CLI reads and writes.
#[must_use]
pub fn records_to_jsonl(records: &[Value]) -> String {
    records
        .iter()
        .map(|record| serde_json::to_string(record).expect("corpus records serialize"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_matches_the_epoch_and_known_instants() {
        assert_eq!(iso8601_utc(0).expect("epoch"), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            iso8601_utc(1_785_593_003_104).expect("known instant"),
            "2026-08-01T14:03:23.104Z"
        );
        // Negative stamps count backward from the epoch, as div/rem-euclid
        // intends: -1 ms is the last millisecond of 1969.
        assert_eq!(
            iso8601_utc(-1).expect("negative"),
            "1969-12-31T23:59:59.999Z"
        );
    }

    #[test]
    fn conversion_round_trips_through_the_corpus_decoder() {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("testdata")
                .join("sagashop")
                .join("ts-happy.json"),
        )
        .expect("vendored capture is readable");
        let capture = Capture::from_json(&bytes).expect("vendored capture decodes");
        let records = capture_to_corpus_records(&capture).expect("capture converts");
        assert_eq!(records.len(), capture.spans.len());

        let jsonl = records_to_jsonl(&records);
        let corpus = sentinel_ingest::corpus::Corpus::from_jsonl(jsonl.as_bytes())
            .expect("converted corpus decodes");
        assert_eq!(corpus.records.len(), capture.spans.len());

        // Every record's name is the analysis vocabulary, not the raw route.
        // The SERVER record — the payment service's own view — is the one
        // carrying the sagashop.* evidence.
        let charge = corpus
            .records
            .iter()
            .find(|record| record.name == "payment.charge" && record.kind == "SERVER")
            .expect("the payment charge record keeps its event name");
        assert_eq!(
            charge.text_attribute("service.name"),
            Some("sagashop-payment")
        );

        // The happy capture's charge->fulfill transition edge rides as a link.
        // The link names the *order-service* charge span (the transition's
        // author), not the payment-service charge this test found first.
        let fulfill = corpus
            .records
            .iter()
            .find(|record| record.name == "order.fulfill" && record.kind == "SERVER")
            .expect("the fulfill record exists");
        let order_charge = corpus
            .records
            .iter()
            .find(|record| record.name == "order.charge" && record.kind == "SERVER")
            .expect("the order-service charge record exists");
        assert!(
            fulfill
                .links
                .iter()
                .any(|link| link.span_id == order_charge.id),
            "the charge-to-fulfill transition edge is encoded as a link"
        );
    }
}
