//! Untrusted `capture-envelope-v1` decoding into plain, structurally validated DTOs.
//!
//! This module is the ingest crate's analogue of [`sentinel_model::wire`]: it is
//! the untrusted-storage boundary, and nothing downstream of it may assume a
//! field exists, has the right JSON type, or is in range.
//!
//! `serde`'s derive machinery is deliberately **not** used. The whole point of
//! this boundary is that structural validation is explicit, reviewable code:
//! every accepted field is named in [`Capture::from_json`] and its neighbours,
//! and every unknown field is rejected by [`reject_unknown_fields`] rather than
//! by an attribute a future edit could silently drop.
//!
//! # What is deliberately *not* ported from `shared/capture.ts`
//!
//! The reference validator rejects a span whose `logicalParentSpanId` names a
//! span that is not in the capture. This decoder does **not**: an unresolved
//! parent is *evidence about missing evidence*, and ADR-0019 names "missing
//! parents" as a case the tracer bullet must handle rather than refuse. The
//! Adapter turns it into a dangling causal edge, and the Core's evidence gate
//! turns that into an honest Inconclusive. Rejecting it here would make that
//! path untestable.

#![allow(clippy::doc_markdown)] // Product names (SagaShop, OpenTelemetry) read as prose here.
#![allow(clippy::module_name_repetitions)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use serde_json::{Map, Value};

/// The only artifact version this decoder accepts.
pub const ARTIFACT_VERSION: &str = "capture-envelope-v1";
/// The only SagaShop contract version this decoder accepts.
pub const CONTRACT_VERSION: &str = "sagashop-contract-v1";

/// Structural failure decoding an untrusted capture artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CaptureError {
    /// The bytes were not valid JSON.
    Malformed(String),
    /// A required member is absent.
    MissingField {
        /// Dotted path of the absent member.
        path: String,
    },
    /// A member had the wrong JSON type or was out of range.
    InvalidField {
        /// Dotted path of the offending member.
        path: String,
        /// Why the value is unacceptable.
        detail: String,
    },
    /// A member outside the schema was present.
    UnknownField {
        /// Dotted path of the offending member.
        path: String,
    },
    /// The artifact declares a version this decoder does not implement.
    UnsupportedVersion {
        /// Which version stamp mismatched.
        field: &'static str,
        /// Version the decoder implements.
        expected: &'static str,
        /// Version the artifact declared.
        actual: String,
    },
}

impl Display for CaptureError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(detail) => write!(formatter, "capture is not valid JSON: {detail}"),
            Self::MissingField { path } => {
                write!(formatter, "required capture member is missing: {path}")
            }
            Self::InvalidField { path, detail } => {
                write!(formatter, "invalid capture member {path}: {detail}")
            }
            Self::UnknownField { path } => {
                write!(formatter, "capture member is outside the schema: {path}")
            }
            Self::UnsupportedVersion {
                field,
                expected,
                actual,
            } => write!(
                formatter,
                "unsupported {field}: expected {expected}, found {actual}"
            ),
        }
    }
}

impl Error for CaptureError {}

fn missing(path: impl Into<String>) -> CaptureError {
    CaptureError::MissingField { path: path.into() }
}

fn invalid(path: impl Into<String>, detail: impl Into<String>) -> CaptureError {
    CaptureError::InvalidField {
        path: path.into(),
        detail: detail.into(),
    }
}

fn reject_unknown_fields(
    object: &Map<String, Value>,
    known: &[&str],
    path: &str,
) -> Result<(), CaptureError> {
    for key in object.keys() {
        if !known.contains(&key.as_str()) {
            return Err(CaptureError::UnknownField {
                path: format!("{path}.{key}"),
            });
        }
    }
    Ok(())
}

fn object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>, CaptureError> {
    value
        .as_object()
        .ok_or_else(|| invalid(path, "expected a JSON object"))
}

fn member<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<&'a Value, CaptureError> {
    object
        .get(key)
        .ok_or_else(|| missing(format!("{path}.{key}")))
}

fn text(object: &Map<String, Value>, key: &str, path: &str) -> Result<String, CaptureError> {
    let field_path = format!("{path}.{key}");
    let value = member(object, key, path)?
        .as_str()
        .ok_or_else(|| invalid(&field_path, "expected a JSON string"))?;
    if value.is_empty() {
        return Err(invalid(&field_path, "expected a non-empty JSON string"));
    }
    Ok(value.to_owned())
}

fn optional_text(
    object: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<Option<String>, CaptureError> {
    let field_path = format!("{path}.{key}");
    match member(object, key, path)? {
        Value::Null => Ok(None),
        Value::String(value) if !value.is_empty() => Ok(Some(value.clone())),
        _ => Err(invalid(
            field_path,
            "expected null or a non-empty JSON string",
        )),
    }
}

fn milliseconds(object: &Map<String, Value>, key: &str, path: &str) -> Result<i64, CaptureError> {
    let field_path = format!("{path}.{key}");
    let value = member(object, key, path)?
        .as_f64()
        .ok_or_else(|| invalid(&field_path, "expected a JSON number"))?;
    if !value.is_finite() || value < 0.0 || value.fract() != 0.0 {
        return Err(invalid(
            &field_path,
            "expected a non-negative whole millisecond count",
        ));
    }
    // `capture-envelope-v1` caps every millisecond field at 2^53 - 1.
    if value > 9_007_199_254_740_991.0 {
        return Err(invalid(&field_path, "millisecond count exceeds 2^53 - 1"));
    }
    #[allow(clippy::cast_possible_truncation)]
    Ok(value as i64)
}

fn sequence(
    object: &Map<String, Value>,
    key: &str,
    path: &str,
    maximum: u64,
) -> Result<u64, CaptureError> {
    let field_path = format!("{path}.{key}");
    let value = member(object, key, path)?
        .as_u64()
        .ok_or_else(|| invalid(&field_path, "expected a non-negative JSON integer"))?;
    if value < 1 || value > maximum {
        return Err(invalid(
            &field_path,
            format!("expected an integer in 1..={maximum}"),
        ));
    }
    Ok(value)
}

fn hex_id(value: &str, width: usize, path: &str) -> Result<(), CaptureError> {
    if value.len() != width || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid(
            path,
            format!("expected {width} lowercase hexadecimal digits"),
        ));
    }
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(invalid(path, "expected lowercase hexadecimal digits"));
    }
    Ok(())
}

/// A captured attribute value, mirroring the schema's recursive value type.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CaptureAttribute {
    /// JSON `null`.
    Null,
    /// JSON boolean.
    Bool(bool),
    /// JSON string.
    String(String),
    /// JSON number, always finite.
    Number(f64),
    /// JSON array, in artifact order.
    Array(Vec<CaptureAttribute>),
    /// JSON object, keyed deterministically.
    Object(BTreeMap<String, CaptureAttribute>),
}

impl CaptureAttribute {
    fn from_json(value: &Value, path: &str) -> Result<Self, CaptureError> {
        match value {
            Value::Null => Ok(Self::Null),
            Value::Bool(value) => Ok(Self::Bool(*value)),
            Value::String(value) => Ok(Self::String(value.clone())),
            Value::Number(value) => {
                let value = value
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| invalid(path, "expected a finite JSON number"))?;
                Ok(Self::Number(value))
            }
            Value::Array(values) => values
                .iter()
                .enumerate()
                .map(|(index, value)| Self::from_json(value, &format!("{path}[{index}]")))
                .collect::<Result<Vec<_>, _>>()
                .map(Self::Array),
            Value::Object(entries) => {
                let mut values = BTreeMap::new();
                for (key, value) in entries {
                    if key.is_empty() {
                        return Err(invalid(path, "attribute keys must not be empty"));
                    }
                    values.insert(
                        key.clone(),
                        Self::from_json(value, &format!("{path}.{key}"))?,
                    );
                }
                Ok(Self::Object(values))
            }
        }
    }

    /// Returns the value when it is a string.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }
}

fn attributes(
    object: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<BTreeMap<String, CaptureAttribute>, CaptureError> {
    let field_path = format!("{path}.{key}");
    let entries = member(object, key, path)?
        .as_object()
        .ok_or_else(|| invalid(&field_path, "expected a JSON object"))?;
    let mut values = BTreeMap::new();
    for (key, value) in entries {
        if key.is_empty() {
            return Err(invalid(&field_path, "attribute keys must not be empty"));
        }
        values.insert(
            key.clone(),
            CaptureAttribute::from_json(value, &format!("{field_path}.{key}"))?,
        );
    }
    Ok(values)
}

/// Span kind as recorded by the capture artifact.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum SpanKind {
    /// Handles an inbound call.
    Server,
    /// Issues an outbound call.
    Client,
    /// Neither inbound nor outbound.
    Internal,
}

impl SpanKind {
    fn parse(value: &str, path: &str) -> Result<Self, CaptureError> {
        match value {
            "SERVER" => Ok(Self::Server),
            "CLIENT" => Ok(Self::Client),
            "INTERNAL" => Ok(Self::Internal),
            other => Err(invalid(path, format!("unknown span kind {other}"))),
        }
    }
}

/// A point event recorded inside a captured span.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CapturedSpanEvent {
    /// Event name.
    pub name: String,
    /// Producer wall-clock timestamp in milliseconds.
    pub timestamp_ms: i64,
    /// Event attributes.
    pub attributes: BTreeMap<String, CaptureAttribute>,
}

const SPAN_EVENT_FIELDS: &[&str] = &["name", "timestamp", "attributes"];

impl CapturedSpanEvent {
    fn from_json(value: &Value, path: &str) -> Result<Self, CaptureError> {
        let entries = object(value, path)?;
        reject_unknown_fields(entries, SPAN_EVENT_FIELDS, path)?;
        Ok(Self {
            name: text(entries, "name", path)?,
            timestamp_ms: milliseconds(entries, "timestamp", path)?,
            attributes: attributes(entries, "attributes", path)?,
        })
    }
}

/// One span of a `capture-envelope-v1` artifact, structurally validated only.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CapturedSpan {
    /// Raw OpenTelemetry trace id.
    pub trace_id: String,
    /// Raw OpenTelemetry span id.
    pub span_id: String,
    /// Raw OpenTelemetry parent span id.
    pub parent_span_id: Option<String>,
    /// Emitting SagaShop service.
    pub service: String,
    /// Span name.
    pub name: String,
    /// Span kind.
    pub kind: SpanKind,
    /// Span attributes.
    pub attributes: BTreeMap<String, CaptureAttribute>,
    /// Span status.
    pub status: String,
    /// Point events recorded inside the span.
    pub events: Vec<CapturedSpanEvent>,
    /// Producer wall-clock span start in milliseconds. Diagnostics only.
    pub start_ms: i64,
    /// Producer wall-clock span end in milliseconds. Diagnostics only.
    pub end_ms: i64,
    /// Producer wall-clock span duration in milliseconds. Diagnostics only.
    pub duration_ms: i64,
    /// Seed-derived stable trace id.
    pub logical_trace_id: String,
    /// Seed-derived stable span id.
    pub logical_span_id: String,
    /// Seed-derived stable parent span id.
    pub logical_parent_span_id: Option<String>,
    /// Per-service serialization order, contiguous from one. Not a clock.
    pub source_sequence: u64,
    /// Trace-local ancestry depth. Not a clock.
    pub causal_sequence: u64,
}

const SPAN_FIELDS: &[&str] = &[
    "traceId",
    "spanId",
    "parentSpanId",
    "service",
    "name",
    "kind",
    "attributes",
    "status",
    "events",
    "startMs",
    "endMs",
    "durMs",
    "logicalTraceId",
    "logicalSpanId",
    "logicalParentSpanId",
    "sourceSequence",
    "causalSequence",
];

impl CapturedSpan {
    fn from_json(value: &Value, path: &str) -> Result<Self, CaptureError> {
        let entries = object(value, path)?;
        reject_unknown_fields(entries, SPAN_FIELDS, path)?;

        let trace_id = text(entries, "traceId", path)?;
        hex_id(&trace_id, 32, &format!("{path}.traceId"))?;
        let span_id = text(entries, "spanId", path)?;
        hex_id(&span_id, 16, &format!("{path}.spanId"))?;
        let parent_span_id = optional_text(entries, "parentSpanId", path)?;
        if let Some(parent) = &parent_span_id {
            hex_id(parent, 16, &format!("{path}.parentSpanId"))?;
        }
        let logical_trace_id = text(entries, "logicalTraceId", path)?;
        hex_id(&logical_trace_id, 32, &format!("{path}.logicalTraceId"))?;
        let logical_span_id = text(entries, "logicalSpanId", path)?;
        hex_id(&logical_span_id, 16, &format!("{path}.logicalSpanId"))?;
        let logical_parent_span_id = optional_text(entries, "logicalParentSpanId", path)?;
        if let Some(parent) = &logical_parent_span_id {
            hex_id(parent, 16, &format!("{path}.logicalParentSpanId"))?;
        }

        let kind = SpanKind::parse(&text(entries, "kind", path)?, &format!("{path}.kind"))?;
        let status = text(entries, "status", path)?;
        if !matches!(status.as_str(), "unset" | "error" | "ok") {
            return Err(invalid(
                format!("{path}.status"),
                format!("unknown span status {status}"),
            ));
        }

        let events_path = format!("{path}.events");
        let events = member(entries, "events", path)?
            .as_array()
            .ok_or_else(|| invalid(&events_path, "expected a JSON array"))?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                CapturedSpanEvent::from_json(value, &format!("{events_path}[{index}]"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let start_ms = milliseconds(entries, "startMs", path)?;
        let end_ms = milliseconds(entries, "endMs", path)?;
        let duration_ms = milliseconds(entries, "durMs", path)?;
        if end_ms < start_ms {
            return Err(invalid(
                format!("{path}.endMs"),
                "span end must not precede span start",
            ));
        }

        Ok(Self {
            trace_id,
            span_id,
            parent_span_id,
            service: text(entries, "service", path)?,
            name: text(entries, "name", path)?,
            kind,
            attributes: attributes(entries, "attributes", path)?,
            status,
            events,
            start_ms,
            end_ms,
            duration_ms,
            logical_trace_id,
            logical_span_id,
            logical_parent_span_id,
            source_sequence: sequence(entries, "sourceSequence", path, 100_000)?,
            causal_sequence: sequence(entries, "causalSequence", path, 256)?,
        })
    }

    /// Returns a `sagashop.*` string attribute, when present.
    #[must_use]
    pub fn sagashop_text(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).and_then(CaptureAttribute::as_str)
    }
}

/// A structurally validated `capture-envelope-v1` artifact.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Capture {
    /// Emitting SagaShop port, e.g. `ts`.
    pub port: String,
    /// Scenario name, e.g. `happy`.
    pub scenario: String,
    /// Seed the capture was generated with.
    pub seed: String,
    /// Spans in artifact order.
    pub spans: Vec<CapturedSpan>,
}

const CAPTURE_FIELDS: &[&str] = &[
    "artifactVersion",
    "contractVersion",
    "port",
    "scenario",
    "seed",
    "spans",
];

impl Capture {
    /// Decodes and structurally validates an untrusted capture artifact.
    ///
    /// Both version stamps must match this decoder exactly, unknown members are
    /// rejected, and every field is range checked. Semantic checks that belong
    /// to the Adapter — sequence contiguity, causal resolution — are not
    /// performed here; see the module documentation for why unresolved logical
    /// parents in particular are accepted as evidence rather than rejected.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError`] for malformed JSON, a missing or ill-typed
    /// member, an unknown member, or an artifact or contract version this
    /// decoder does not implement.
    pub fn from_json(bytes: &[u8]) -> Result<Self, CaptureError> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|error| CaptureError::Malformed(error.to_string()))?;
        let entries = object(&value, "capture")?;
        reject_unknown_fields(entries, CAPTURE_FIELDS, "capture")?;

        let artifact_version = text(entries, "artifactVersion", "capture")?;
        if artifact_version != ARTIFACT_VERSION {
            return Err(CaptureError::UnsupportedVersion {
                field: "artifactVersion",
                expected: ARTIFACT_VERSION,
                actual: artifact_version,
            });
        }
        let contract_version = text(entries, "contractVersion", "capture")?;
        if contract_version != CONTRACT_VERSION {
            return Err(CaptureError::UnsupportedVersion {
                field: "contractVersion",
                expected: CONTRACT_VERSION,
                actual: contract_version,
            });
        }

        let spans = member(entries, "spans", "capture")?
            .as_array()
            .ok_or_else(|| invalid("capture.spans", "expected a JSON array"))?;
        if spans.is_empty() {
            return Err(invalid("capture.spans", "expected at least one span"));
        }
        let spans = spans
            .iter()
            .enumerate()
            .map(|(index, value)| {
                CapturedSpan::from_json(value, &format!("capture.spans[{index}]"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            port: text(entries, "port", "capture")?,
            scenario: text(entries, "scenario", "capture")?,
            seed: text(entries, "seed", "capture")?,
            spans,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_span() -> String {
        r#"{
            "traceId": "13912439bfec6d828fe6726a5ce24aed",
            "spanId": "faa6fcd1465726d0",
            "parentSpanId": null,
            "service": "sagashop-driver",
            "name": "scenario.happy",
            "kind": "INTERNAL",
            "attributes": { "sagashop.scenario": "happy" },
            "status": "unset",
            "events": [],
            "startMs": 0,
            "endMs": 0,
            "durMs": 0,
            "logicalTraceId": "13912439bfec6d828fe6726a5ce24aed",
            "logicalSpanId": "faa6fcd1465726d0",
            "logicalParentSpanId": null,
            "sourceSequence": 1,
            "causalSequence": 1
        }"#
        .to_owned()
    }

    fn capture_with(span: &str) -> String {
        format!(
            r#"{{"artifactVersion":"capture-envelope-v1","contractVersion":"sagashop-contract-v1",
                 "port":"ts","scenario":"happy","seed":"s","spans":[{span}]}}"#
        )
    }

    #[test]
    fn minimal_capture_decodes() {
        let capture = Capture::from_json(capture_with(&minimal_span()).as_bytes())
            .expect("minimal capture decodes");
        assert_eq!(capture.port, "ts");
        assert_eq!(capture.spans.len(), 1);
        assert_eq!(capture.spans[0].kind, SpanKind::Internal);
        assert_eq!(
            capture.spans[0].sagashop_text("sagashop.scenario"),
            Some("happy")
        );
    }

    #[test]
    fn unknown_members_are_rejected_at_every_level() {
        let capture =
            capture_with(&minimal_span()).replace(r#""port":"ts""#, r#""port":"ts","extra":1"#);
        assert_eq!(
            Capture::from_json(capture.as_bytes()),
            Err(CaptureError::UnknownField {
                path: "capture.extra".to_owned()
            })
        );

        let span =
            minimal_span().replace(r#""status": "unset""#, r#""status": "unset", "extra": 1"#);
        assert_eq!(
            Capture::from_json(capture_with(&span).as_bytes()),
            Err(CaptureError::UnknownField {
                path: "capture.spans[0].extra".to_owned()
            })
        );
    }

    #[test]
    fn unresolved_logical_parents_are_accepted_as_evidence() {
        let span = minimal_span().replace(
            r#""logicalParentSpanId": null"#,
            r#""logicalParentSpanId": "0000000000000000""#,
        );
        let capture = Capture::from_json(capture_with(&span).as_bytes())
            .expect("dangling parent is evidence");
        assert_eq!(
            capture.spans[0].logical_parent_span_id.as_deref(),
            Some("0000000000000000")
        );
    }

    #[test]
    fn malformed_identifiers_and_ranges_are_rejected() {
        let span = minimal_span().replace(r#""sourceSequence": 1"#, r#""sourceSequence": 0"#);
        assert!(matches!(
            Capture::from_json(capture_with(&span).as_bytes()),
            Err(CaptureError::InvalidField { .. })
        ));

        let span = minimal_span().replace(
            r#""logicalSpanId": "faa6fcd1465726d0""#,
            r#""logicalSpanId": "FAA6FCD1465726D0""#,
        );
        assert!(matches!(
            Capture::from_json(capture_with(&span).as_bytes()),
            Err(CaptureError::InvalidField { .. })
        ));
    }
}
