//! Untrusted `trace-corpus-v1` decoding into plain, structurally validated DTOs.
//!
//! `trace-corpus-v1` is the output of `trace-grab` — the contributor-side
//! sanitizer partners run on their own machines (trace-grab ADR-0004/0010) —
//! and it arrives here exactly the way `capture-envelope-v1` does: as an
//! **untrusted input** from outside Sentinel's trust boundary. This module is
//! that boundary's decoder, the structural twin of [`crate::capture`]: the
//! corpus is walked as a `serde_json::Value` by hand, `serde`'s derive
//! machinery is deliberately not used, every accepted field is named here, and
//! every field outside the schema is rejected by [`reject_unknown_fields`]
//! rather than silently bagged. The one sanctioned bag is the schema's own
//! `unmapped` member, which is carried verbatim and never interpreted.
//!
//! # One decoder, two value spaces
//!
//! SCHEMA.md defines two structurally identical value spaces: the generic
//! JSONL *input* (vendor strings) and the *corpus* (the same records with
//! string values replaced by opaque `TOK_<10 hex>` tokens). This decoder
//! accepts both, because they differ only in string *values* and the decoder
//! validates *structure*. Identifier fields therefore accept any non-empty
//! string: pre-sanitization ids are vendor-shaped (UUIDs, hex, opaque vendor
//! ids), post-sanitization ids are tokens, and only equality between them is
//! ever load-bearing downstream. Requiring a specific shape would reject one
//! of the two value spaces this decoder exists to accept.
//!
//! # What is deliberately *not* validated
//!
//! - **Dangling `parent_id` values are accepted**, exactly as unresolved
//!   `logicalParentSpanId` values are in [`crate::capture`]: a missing parent
//!   is *evidence about missing evidence* (ADR-0019), and the Adapter turns it
//!   into a dangling causal edge rather than an error. trace-grab ADR-0010
//!   preserves dangling parents for the same reason, and its manifest counts
//!   them so they are visible rather than silent.
//! - **No cross-record checks at all.** Uniqueness of `id`, referential
//!   integrity of `parent_id` and `links[]`, and trace grouping are semantic
//!   properties of a *bundle*, and the Adapter is where bundle-level evidence
//!   is composed. This decoder validates one line at a time and fails with
//!   that line's number in the error.
//! - **The manifest is not decoded.** It is bundle metadata (counts, hashes,
//!   policy provenance) rather than per-record evidence; nothing in the
//!   Adapter or the Core consumes it in this slice. When corpus provenance
//!   becomes load-bearing — the corpus SHA-256 binding a bundle to its
//!   manifest, say — it gets its own decoder beside this one.
//!
//! # Timestamps
//!
//! `start` and `end` are ISO-8601 UTC strings at whatever sub-second precision
//! the vendor gave. [`parse_rfc3339_utc`] accepts exactly the
//! `YYYY-MM-DDTHH:MM:SS[.fraction]Z` shape and retains the parsed precision,
//! because the Adapter later reports that precision as the occurrence time's
//! uncertainty — an honestly coarse clock, not a falsely exact one. Anything
//! else (offsets other than `Z`, missing `T`, impossible dates, leap-second
//! `:60`) is a structural error naming the offending path.

#![allow(clippy::doc_markdown)] // Product names (trace-grab, LangSmith) read as prose here.
#![allow(clippy::module_name_repetitions)]

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use serde_json::{Map, Value};

/// The only corpus schema version this decoder accepts.
pub const SCHEMA_VERSION: &str = "trace-corpus-v1";

/// Structural failure decoding an untrusted corpus line.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CorpusError {
    /// The bytes of one line were not valid JSON.
    Malformed {
        /// One-based line number in the corpus file.
        line: usize,
        /// The underlying parse failure.
        detail: String,
    },
    /// A required member is absent.
    MissingField {
        /// One-based line number in the corpus file.
        line: usize,
        /// Dotted path of the absent member.
        path: String,
    },
    /// A member had the wrong JSON type or an out-of-range value.
    InvalidField {
        /// One-based line number in the corpus file.
        line: usize,
        /// Dotted path of the offending member.
        path: String,
        /// Why the value is unacceptable.
        detail: String,
    },
    /// A member outside the schema was present.
    UnknownField {
        /// One-based line number in the corpus file.
        line: usize,
        /// Dotted path of the offending member.
        path: String,
    },
    /// The corpus was empty or contained no decodable records.
    Empty,
}

impl Display for CorpusError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { line, detail } => {
                write!(formatter, "corpus line {line} is not valid JSON: {detail}")
            }
            Self::MissingField { line, path } => {
                write!(
                    formatter,
                    "corpus line {line}: required member is missing: {path}"
                )
            }
            Self::InvalidField { line, path, detail } => {
                write!(
                    formatter,
                    "corpus line {line}: invalid member {path}: {detail}"
                )
            }
            Self::UnknownField { line, path } => write!(
                formatter,
                "corpus line {line}: member is outside the schema: {path}"
            ),
            Self::Empty => formatter.write_str("corpus contains no records"),
        }
    }
}

impl Error for CorpusError {}

fn missing(line: usize, path: impl Into<String>) -> CorpusError {
    CorpusError::MissingField {
        line,
        path: path.into(),
    }
}

fn invalid(line: usize, path: impl Into<String>, detail: impl Into<String>) -> CorpusError {
    CorpusError::InvalidField {
        line,
        path: path.into(),
        detail: detail.into(),
    }
}

fn reject_unknown_fields(
    line: usize,
    object: &Map<String, Value>,
    known: &[&str],
    path: &str,
) -> Result<(), CorpusError> {
    for key in object.keys() {
        if !known.contains(&key.as_str()) {
            return Err(CorpusError::UnknownField {
                line,
                path: format!("{path}.{key}"),
            });
        }
    }
    Ok(())
}

fn object<'a>(
    value: &'a Value,
    line: usize,
    path: &str,
) -> Result<&'a Map<String, Value>, CorpusError> {
    value
        .as_object()
        .ok_or_else(|| invalid(line, path, "expected a JSON object"))
}

fn member<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    line: usize,
    path: &str,
) -> Result<&'a Value, CorpusError> {
    object
        .get(key)
        .ok_or_else(|| missing(line, format!("{path}.{key}")))
}

fn text(
    object: &Map<String, Value>,
    key: &str,
    line: usize,
    path: &str,
) -> Result<String, CorpusError> {
    let field_path = format!("{path}.{key}");
    let value = member(object, key, line, path)?
        .as_str()
        .ok_or_else(|| invalid(line, &field_path, "expected a JSON string"))?;
    if value.is_empty() {
        return Err(invalid(
            line,
            field_path,
            "expected a non-empty JSON string",
        ));
    }
    Ok(value.to_owned())
}

/// Reads a member that must be a string or `null`, as `parent_id` and `end` are.
fn optional_text(
    object: &Map<String, Value>,
    key: &str,
    line: usize,
    path: &str,
) -> Result<Option<String>, CorpusError> {
    let field_path = format!("{path}.{key}");
    match member(object, key, line, path)? {
        Value::Null => Ok(None),
        Value::String(value) if !value.is_empty() => Ok(Some(value.clone())),
        _ => Err(invalid(
            line,
            field_path,
            "expected null or a non-empty JSON string",
        )),
    }
}

/// Reads a member that must be a string, empty allowed, as free text is.
///
/// `error.message` and `labels[].comment` are plain strings under SCHEMA.md,
/// and the input value space is vendor-shaped: an empty message is valid.
/// Identifiers and vocabulary (which become Event ids and kinds) stay
/// non-empty via [`text`], because the model would reject them downstream
/// and a precise decode error beats an adaptation failure.
fn free_text(
    object: &Map<String, Value>,
    key: &str,
    line: usize,
    path: &str,
) -> Result<String, CorpusError> {
    let field_path = format!("{path}.{key}");
    member(object, key, line, path)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid(line, field_path, "expected a JSON string"))
}

/// Reads an optional free-text member: `null`, or any string, empty included.
fn optional_free_text(
    object: &Map<String, Value>,
    key: &str,
    line: usize,
    path: &str,
) -> Result<Option<String>, CorpusError> {
    match member(object, key, line, path)? {
        Value::Null => Ok(None),
        Value::String(value) => Ok(Some(value.clone())),
        _ => Err(invalid(
            line,
            format!("{path}.{key}"),
            "expected null or a JSON string",
        )),
    }
}

/// An arbitrary JSON value carried verbatim, as `inputs` and `outputs` are.
///
/// The corpus forbids non-finite numbers — JSON itself cannot express them, so
/// any number that survives `serde_json` parsing is finite by construction.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CorpusValue {
    /// JSON `null`.
    Null,
    /// JSON boolean.
    Bool(bool),
    /// JSON string.
    String(String),
    /// JSON number, always finite.
    Number(f64),
    /// JSON array, in record order.
    Array(Vec<CorpusValue>),
    /// JSON object, keyed deterministically.
    Object(BTreeMap<String, CorpusValue>),
}

impl CorpusValue {
    fn from_json(value: &Value, line: usize, path: &str) -> Result<Self, CorpusError> {
        match value {
            Value::Null => Ok(Self::Null),
            Value::Bool(value) => Ok(Self::Bool(*value)),
            Value::String(value) => Ok(Self::String(value.clone())),
            Value::Number(value) => {
                let value = value
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| invalid(line, path, "expected a finite JSON number"))?;
                Ok(Self::Number(value))
            }
            Value::Array(values) => values
                .iter()
                .enumerate()
                .map(|(index, value)| Self::from_json(value, line, &format!("{path}[{index}]")))
                .collect::<Result<Vec<_>, _>>()
                .map(Self::Array),
            Value::Object(entries) => {
                // Empty keys are allowed in free values: `inputs`, `outputs`
                // and label values are arbitrary vendor JSON, carried
                // verbatim per SCHEMA.md. Only the keyed maps the policy
                // language addresses (attributes, unmapped, links) forbid
                // them, via `keyed_object`.
                let mut values = BTreeMap::new();
                for (key, value) in entries {
                    values.insert(
                        key.clone(),
                        Self::from_json(value, line, &format!("{path}.{key}"))?,
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

fn free_value(
    object: &Map<String, Value>,
    key: &str,
    line: usize,
    path: &str,
) -> Result<CorpusValue, CorpusError> {
    member(object, key, line, path)
        .and_then(|value| CorpusValue::from_json(value, line, &format!("{path}.{key}")))
}

fn keyed_object(
    object: &Map<String, Value>,
    key: &str,
    line: usize,
    path: &str,
) -> Result<BTreeMap<String, CorpusValue>, CorpusError> {
    let field_path = format!("{path}.{key}");
    let entries = member(object, key, line, path)?
        .as_object()
        .ok_or_else(|| invalid(line, &field_path, "expected a JSON object"))?;
    let mut values = BTreeMap::new();
    for (key, value) in entries {
        if key.is_empty() {
            return Err(invalid(line, field_path, "keys must not be empty"));
        }
        values.insert(
            key.clone(),
            CorpusValue::from_json(value, line, &format!("{field_path}.{key}"))?,
        );
    }
    Ok(values)
}

/// A normalized record status on OTLP's three-value model.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RecordStatus {
    /// The vendor recorded no status.
    Unset,
    /// The unit of work succeeded.
    Ok,
    /// The unit of work failed.
    Error,
}

impl RecordStatus {
    /// Returns the wire representation SCHEMA.md defines.
    #[must_use]
    pub const fn to_str(self) -> &'static str {
        match self {
            Self::Unset => "unset",
            Self::Ok => "ok",
            Self::Error => "error",
        }
    }

    fn parse(value: &str, line: usize, path: &str) -> Result<Self, CorpusError> {
        match value {
            "unset" => Ok(Self::Unset),
            "ok" => Ok(Self::Ok),
            "error" => Ok(Self::Error),
            other => Err(invalid(
                line,
                path,
                format!("unknown record status {other}"),
            )),
        }
    }
}

/// An ISO-8601 UTC timestamp split into unix nanoseconds and its precision.
///
/// The precision is retained because the Adapter reports it as the occurrence
/// time's uncertainty: a vendor that stamps milliseconds gets a millisecond
/// uncertainty, not a false zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CorpusTimestamp {
    /// Seconds since the unix epoch.
    pub unix_seconds: i64,
    /// Sub-second nanoseconds, `0..1_000_000_000`.
    pub nanoseconds: u32,
    /// The timestamp's own resolution in nanoseconds: `1` for whole seconds,
    /// `1_000` for milliseconds, `1_000_000_000` when only the second is known.
    pub resolution_nanoseconds: u32,
}

impl CorpusTimestamp {
    /// Whole nanoseconds since the unix epoch.
    ///
    /// Saturates rather than panicking on overflow; the corpus dates that
    /// could overflow i128 nanoseconds are not representable in 4-digit years.
    #[must_use]
    pub fn total_nanoseconds(self) -> i128 {
        i128::from(self.unix_seconds)
            .saturating_mul(1_000_000_000)
            .saturating_add(i128::from(self.nanoseconds))
    }
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    // Howard Hinnant's `days_from_civil`, the standard days-from-epoch
    // algorithm; correct for every date the 4-digit-year format can name.
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_shifted = u64::from(if month > 2 { month - 3 } else { month + 9 });
    let day_of_year = (153 * month_shifted + 2) / 5 + u64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100
        + i64::try_from(day_of_year).expect("day_of_year < 366 by construction");
    era * 146_097 + day_of_era - 719_468
}

/// Parses an RFC-3339 UTC timestamp: `YYYY-MM-DDTHH:MM:SS[.f…]Z`.
///
/// Fractional digits beyond nanosecond precision are a structural error rather
/// than a silent truncation: the decoder cannot honestly claim a resolution
/// the string does not carry, and inventing one would misreport the clock's
/// uncertainty downstream.
///
/// The parser is **byte-indexed throughout**. A JSON string is valid UTF-8 but
/// may contain multi-byte characters anywhere, and slicing `&value[a..b]` on a
/// non-char-boundary panics — an untrusted input must produce an error, never
/// a panic, so every field is extracted with byte arithmetic and any byte that
/// is not the expected ASCII separator or digit fails the structural check
/// before any string is sliced.
fn parse_rfc3339_utc(value: &str, line: usize, path: &str) -> Result<CorpusTimestamp, CorpusError> {
    const SHAPE: &str = "expected YYYY-MM-DDTHH:MM:SS[.fraction]Z";
    let bad = |detail: &str| invalid(line, path, detail.to_owned());
    let bytes = value.as_bytes();

    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[bytes.len() - 1] != b'Z'
    {
        return Err(bad(SHAPE));
    }

    // ASCII digit extraction: any multi-byte UTF-8 byte is >= 0x80 and fails
    // `is_ascii_digit`, so a non-ASCII character in any field is a structural
    // error rather than a slicing panic.
    let two_digits = |index: usize| -> Option<u32> {
        match [bytes.get(index), bytes.get(index + 1)] {
            [Some(&high), Some(&low)] if high.is_ascii_digit() && low.is_ascii_digit() => {
                Some(u32::from((high - b'0') * 10 + (low - b'0')))
            }
            _ => None,
        }
    };
    let four_digits = |index: usize| -> Option<i64> {
        (0..4).try_fold(0_i64, |accumulator, offset| {
            bytes
                .get(index + offset)
                .filter(|byte| byte.is_ascii_digit())
                .map(|byte| accumulator * 10 + i64::from(byte - b'0'))
        })
    };

    let Some(year) = four_digits(0) else {
        return Err(bad("expected ASCII digits in every field"));
    };
    let (Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        two_digits(5),
        two_digits(8),
        two_digits(11),
        two_digits(14),
        two_digits(17),
    ) else {
        return Err(bad("expected ASCII digits in every field"));
    };

    if !(1..=12).contains(&month) {
        return Err(bad("month is outside 01..=12"));
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let day_bound = month_days[(month - 1) as usize];
    if !(1..=day_bound).contains(&day) {
        return Err(bad("day is outside the month's range"));
    }
    if hour > 23 {
        return Err(bad("hour is outside 00..=23"));
    }
    if minute > 59 {
        return Err(bad("minute is outside 00..=59"));
    }
    // RFC 3339 permits second 60 for leap seconds; rejecting it keeps the
    // arithmetic total, and no vendor exporter emits one.
    if second > 59 {
        return Err(bad("second is outside 00..=59"));
    }

    // A whole-second timestamp carries one-second resolution — not the
    // nanosecond resolution a bare `0` would claim. The uncertainty reported
    // downstream is the clock's honesty about itself.
    let (nanoseconds, resolution_nanoseconds) = if bytes.len() == 20 {
        (0, 1_000_000_000)
    } else {
        parse_fraction(bytes, line, path)?
    };

    let days = days_from_civil(year, month, day);
    let unix_seconds =
        days * 86_400 + i64::from(hour) * 3_600 + i64::from(minute) * 60 + i64::from(second);
    Ok(CorpusTimestamp {
        unix_seconds,
        nanoseconds,
        resolution_nanoseconds,
    })
}

/// Parses the `[.fraction]Z` tail of an RFC-3339 timestamp into nanoseconds
/// and the resolution those digits honestly carry. Byte-indexed for the same
/// non-panic reason as [`parse_rfc3339_utc`].
fn parse_fraction(bytes: &[u8], line: usize, path: &str) -> Result<(u32, u32), CorpusError> {
    let bad = |detail: &str| invalid(line, path, detail.to_owned());
    // bytes[19] must be the '.'; the final byte is the 'Z' the caller checked.
    if bytes.len() < 22 || bytes[19] != b'.' {
        return Err(bad("expected YYYY-MM-DDTHH:MM:SS[.fraction]Z"));
    }
    let digits = &bytes[20..bytes.len() - 1];
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Err(bad("expected .fraction or Z after seconds"));
    }
    if digits.len() > 9 {
        return Err(bad(
            "fractional precision beyond nanoseconds is not representable",
        ));
    }
    let places = u32::try_from(9 - digits.len())
        .expect("at most nine fractional digits were validated above");
    let scaled = 10_u32.checked_pow(places).expect("at most 10^9");
    // Nine digits cannot exceed 999_999_999, so the fold cannot overflow.
    let nanoseconds = digits
        .iter()
        .fold(0_u32, |accumulator, byte| {
            accumulator * 10 + u32::from(byte - b'0')
        })
        .saturating_mul(scaled)
        .min(999_999_999);
    Ok((nanoseconds, scaled))
}

fn timestamp(
    object: &Map<String, Value>,
    key: &str,
    line: usize,
    path: &str,
) -> Result<CorpusTimestamp, CorpusError> {
    let field_path = format!("{path}.{key}");
    let value = text(object, key, line, path)?;
    parse_rfc3339_utc(&value, line, &field_path)
}

/// A native annotation on a record: `{ key, value, comment }` (trace-grab ADR-0012).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CorpusLabel {
    /// The annotation's key, e.g. `correctness`.
    pub key: String,
    /// The annotation's value, e.g. `correct`.
    pub value: CorpusValue,
    /// Free text a human typed alongside, when present.
    pub comment: Option<String>,
}

const LABEL_FIELDS: &[&str] = &["key", "value", "comment"];

impl CorpusLabel {
    fn from_json(value: &Value, line: usize, path: &str) -> Result<Self, CorpusError> {
        let entries = object(value, line, path)?;
        reject_unknown_fields(line, entries, LABEL_FIELDS, path)?;
        Ok(Self {
            key: text(entries, "key", line, path)?,
            value: free_value(entries, "value", line, path)?,
            comment: optional_free_text(entries, "comment", line, path)?,
        })
    }
}

/// An OTLP-style causal link: `{ trace_id, span_id, attributes }`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CorpusLink {
    /// The linked record's trace identifier.
    pub trace_id: String,
    /// The linked record's identifier.
    pub span_id: String,
    /// Link attributes, vendor-shaped.
    pub attributes: BTreeMap<String, CorpusValue>,
}

const LINK_FIELDS: &[&str] = &["trace_id", "span_id", "attributes"];

impl CorpusLink {
    fn from_json(value: &Value, line: usize, path: &str) -> Result<Self, CorpusError> {
        let entries = object(value, line, path)?;
        reject_unknown_fields(line, entries, LINK_FIELDS, path)?;
        Ok(Self {
            trace_id: text(entries, "trace_id", line, path)?,
            span_id: text(entries, "span_id", line, path)?,
            attributes: keyed_object(entries, "attributes", line, path)?,
        })
    }
}

/// One `trace-corpus-v1` record, structurally validated only.
///
/// Field-for-field the shape SCHEMA.md's record table defines. Nothing here is
/// cross-validated against any other record: that is the Adapter's job, and
/// the gaps between records are evidence, not corruption.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CorpusRecord {
    /// Corpus-unique record identifier. A token post-sanitization.
    pub id: String,
    /// Trace identifier shared by every record in one trace. A token
    /// post-sanitization; equality is what preserves trace grouping.
    pub trace_id: String,
    /// Parent record identifier, or `None` for a root. May name a record
    /// absent from this bundle: a dangling parent is preserved evidence.
    pub parent_id: Option<String>,
    /// Tool, span, or run name as the vendor gave it. Passes sanitization
    /// verbatim; it is the vocabulary analysis is written in.
    pub name: String,
    /// The vendor's own run/span type string. Copied, never classified.
    pub kind: String,
    /// Span start, producer wall clock.
    pub start: CorpusTimestamp,
    /// Span end, or `None` when the vendor recorded no end (in-flight or
    /// truncated).
    pub end: Option<CorpusTimestamp>,
    /// Normalized status on OTLP's three-value model.
    pub status: RecordStatus,
    /// Error detail, present only when the vendor gave one.
    pub error: Option<CorpusErrorDetail>,
    /// Vendor-shaped inputs, unflattened.
    pub inputs: CorpusValue,
    /// Vendor-shaped outputs, unflattened.
    pub outputs: CorpusValue,
    /// Structured, recognized vendor metadata — OTel-style. Keys are the
    /// dotted-path query language analysis and policy speak.
    pub attributes: BTreeMap<String, CorpusValue>,
    /// Native annotations, tokenized comment aside.
    pub labels: Vec<CorpusLabel>,
    /// OTLP-style causal links, copied exactly as the source gave them.
    pub links: Vec<CorpusLink>,
    /// Source fields the normalizer did not recognize, preserved verbatim in
    /// shape. The schema's explicit bag: carried, never interpreted.
    pub unmapped: BTreeMap<String, CorpusValue>,
    /// Which parser produced this record, e.g. `langsmith`, `otlp`.
    pub vendor: String,
}

/// The `error` member of a record: `{ kind, message }`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CorpusErrorDetail {
    /// Normalized error category; passes sanitization verbatim.
    pub kind: String,
    /// Free text; tokenized by sanitization.
    pub message: String,
}

const ERROR_FIELDS: &[&str] = &["kind", "message"];

impl CorpusErrorDetail {
    fn from_json(value: &Value, line: usize, path: &str) -> Result<Self, CorpusError> {
        let entries = object(value, line, path)?;
        reject_unknown_fields(line, entries, ERROR_FIELDS, path)?;
        Ok(Self {
            kind: text(entries, "kind", line, path)?,
            message: free_text(entries, "message", line, path)?,
        })
    }
}

const RECORD_FIELDS: &[&str] = &[
    "id",
    "trace_id",
    "parent_id",
    "name",
    "kind",
    "start",
    "end",
    "status",
    "error",
    "inputs",
    "outputs",
    "attributes",
    "labels",
    "links",
    "unmapped",
    "source",
];

const SOURCE_FIELDS: &[&str] = &["vendor"];

impl CorpusRecord {
    /// Decodes and structurally validates one untrusted corpus line.
    ///
    /// The line's one-based number is reported in every error so a malformed
    /// record in a million-line bundle is findable. Unknown members are
    /// rejected at every level; `unmapped` is the only sanctioned bag.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError`] for malformed JSON, a missing or ill-typed
    /// member, an unknown member, an unparseable timestamp, or an out-of-range
    /// calendar component.
    pub fn from_json_line(bytes: &[u8], line: usize) -> Result<Self, CorpusError> {
        let value: Value =
            serde_json::from_slice(bytes).map_err(|error| CorpusError::Malformed {
                line,
                detail: error.to_string(),
            })?;
        let entries = object(&value, line, "record")?;
        reject_unknown_fields(line, entries, RECORD_FIELDS, "record")?;

        let status = RecordStatus::parse(
            &text(entries, "status", line, "record")?,
            line,
            "record.status",
        )?;
        let error = match member(entries, "error", line, "record")? {
            Value::Null => None,
            value => Some(CorpusErrorDetail::from_json(value, line, "record.error")?),
        };
        if status != RecordStatus::Error && error.is_some() {
            return Err(invalid(
                line,
                "record.error",
                "error detail is present but status is not \"error\"",
            ));
        }

        let labels_path = "record.labels";
        let labels = member(entries, "labels", line, "record")?
            .as_array()
            .ok_or_else(|| invalid(line, labels_path, "expected a JSON array"))?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                CorpusLabel::from_json(value, line, &format!("{labels_path}[{index}]"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let links_path = "record.links";
        let links = member(entries, "links", line, "record")?
            .as_array()
            .ok_or_else(|| invalid(line, links_path, "expected a JSON array"))?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                CorpusLink::from_json(value, line, &format!("{links_path}[{index}]"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let source = object(
            member(entries, "source", line, "record")?,
            line,
            "record.source",
        )?;
        reject_unknown_fields(line, source, SOURCE_FIELDS, "record.source")?;

        Ok(Self {
            id: text(entries, "id", line, "record")?,
            trace_id: text(entries, "trace_id", line, "record")?,
            parent_id: optional_text(entries, "parent_id", line, "record")?,
            name: text(entries, "name", line, "record")?,
            kind: text(entries, "kind", line, "record")?,
            start: timestamp(entries, "start", line, "record")?,
            end: optional_text(entries, "end", line, "record")?
                .map(|value| parse_rfc3339_utc(&value, line, "record.end"))
                .transpose()?,
            status,
            error,
            inputs: free_value(entries, "inputs", line, "record")?,
            outputs: free_value(entries, "outputs", line, "record")?,
            attributes: keyed_object(entries, "attributes", line, "record")?,
            labels,
            links,
            unmapped: keyed_object(entries, "unmapped", line, "record")?,
            vendor: text(source, "vendor", line, "record.source")?,
        })
    }

    /// Returns a string attribute, when present.
    #[must_use]
    pub fn text_attribute(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).and_then(CorpusValue::as_str)
    }
}

/// A structurally validated `trace-corpus-v1` bundle: its records, in file order.
///
/// The manifest is deliberately absent (see the module documentation). File
/// order is preserved because it is the bundle's only total order, and the
/// Adapter derives per-source delivery sequences from it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Corpus {
    /// Records in corpus file order.
    pub records: Vec<CorpusRecord>,
}

impl Corpus {
    /// Decodes and structurally validates an untrusted `corpus.jsonl` file.
    ///
    /// Blank lines are skipped: JSONL producers commonly terminate the last
    /// line with a newline, and a trailing newline is not a record. Any other
    /// blank line is tolerated for the same reason — it carries no evidence.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError`] naming the one-based line number of the first
    /// structurally invalid record, or [`CorpusError::Empty`] when the file
    /// contains no records at all.
    pub fn from_jsonl(bytes: &[u8]) -> Result<Self, CorpusError> {
        let mut records = Vec::new();
        // UTF-8 is validated per line so an invalid byte is reported with the
        // line it sits on, honoring the error contract the whole decoder
        // keeps: a malformed record in a million-line bundle is findable.
        for (index, line) in bytes.split(|&byte| byte == b'\n').enumerate() {
            let line_number = index + 1;
            let line = std::str::from_utf8(line).map_err(|error| CorpusError::Malformed {
                line: line_number,
                detail: format!("corpus line is not valid UTF-8: {error}"),
            })?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            records.push(CorpusRecord::from_json_line(
                trimmed.as_bytes(),
                line_number,
            )?);
        }
        if records.is_empty() {
            return Err(CorpusError::Empty);
        }
        Ok(Self { records })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_json() -> String {
        r#"{
            "id": "TOK_4f19a2e8c3",
            "trace_id": "TOK_9b7e0d1a5c",
            "parent_id": "TOK_2c8a4f6b91",
            "name": "authenticate_user",
            "kind": "tool",
            "start": "2026-08-01T14:03:22.104Z",
            "end": "2026-08-01T14:03:22.481Z",
            "status": "ok",
            "error": null,
            "inputs": { "user_id": "TOK_a3d17c9e02", "session_token": "TOK_6e5b28f4a1" },
            "outputs": { "authenticated": true, "account_id": "TOK_d091ef3b7c" },
            "attributes": { "service.name": "sagashop-payment" },
            "labels": [{ "key": "correctness", "value": "correct", "comment": null }],
            "links": [],
            "unmapped": { "tags": ["TOK_c4a819e3f0"] },
            "source": { "vendor": "sagashop" }
        }"#
        .to_owned()
    }

    /// Compacts the pretty-printed fixture into the single line JSONL requires;
    /// readability stays in the fixture, machine shape in the file.
    fn compact(record: &str) -> String {
        serde_json::from_str::<Value>(record)
            .and_then(|value| serde_json::to_string(&value))
            .expect("fixture is valid JSON")
    }

    fn corpus_with(record: &str) -> Vec<u8> {
        format!("{}\n", compact(record)).into_bytes()
    }

    #[test]
    fn minimal_record_decodes() {
        let corpus =
            Corpus::from_jsonl(&corpus_with(&compact(&record_json()))).expect("record decodes");
        assert_eq!(corpus.records.len(), 1);
        let record = &corpus.records[0];
        assert_eq!(record.id, "TOK_4f19a2e8c3");
        assert_eq!(record.parent_id.as_deref(), Some("TOK_2c8a4f6b91"));
        assert_eq!(record.status, RecordStatus::Ok);
        assert_eq!(record.error, None);
        assert_eq!(
            record.text_attribute("service.name"),
            Some("sagashop-payment")
        );
        assert_eq!(record.labels.len(), 1);
        assert_eq!(record.labels[0].key, "correctness");
        // 104 ms of sub-second precision: a millisecond-resolution clock.
        assert_eq!(record.start.unix_seconds, 1_785_593_002);
        assert_eq!(record.start.nanoseconds, 104_000_000);
        assert_eq!(record.start.resolution_nanoseconds, 1_000_000);
        assert_eq!(record.vendor, "sagashop");
    }

    #[test]
    fn unknown_members_are_rejected_at_every_level() {
        let record = record_json().replace(
            r#""source": { "vendor": "sagashop" }"#,
            r#""source": { "vendor": "sagashop", "extra": 1 }"#,
        );
        assert_eq!(
            Corpus::from_jsonl(&corpus_with(&record)),
            Err(CorpusError::UnknownField {
                line: 1,
                path: "record.source.extra".to_owned()
            })
        );

        let record = record_json().replace(
            r#""labels": [{ "key": "correctness", "value": "correct", "comment": null }]"#,
            r#""labels": [{ "key": "correctness", "value": "correct", "comment": null, "extra": 1 }]"#,
        );
        assert_eq!(
            Corpus::from_jsonl(&corpus_with(&record)),
            Err(CorpusError::UnknownField {
                line: 1,
                path: "record.labels[0].extra".to_owned()
            })
        );

        let record = record_json().replace(
            r#""links": []"#,
            r#""links": [{ "trace_id": "TOK_1", "span_id": "TOK_2", "attributes": {}, "extra": 1 }]"#,
        );
        assert_eq!(
            Corpus::from_jsonl(&corpus_with(&record)),
            Err(CorpusError::UnknownField {
                line: 1,
                path: "record.links[0].extra".to_owned()
            })
        );
    }

    #[test]
    fn dangling_parent_and_missing_end_are_evidence() {
        let record = record_json().replace(
            r#""parent_id": "TOK_2c8a4f6b91""#,
            r#""parent_id": "TOK_absent0000""#,
        );
        let record = record.replace(r#""end": "2026-08-01T14:03:22.481Z""#, r#""end": null"#);
        let corpus = Corpus::from_jsonl(&corpus_with(&record)).expect("record decodes");
        // A parent no record in this bundle carries, and an unrecorded end,
        // are both preserved rather than repaired or rejected.
        assert_eq!(
            corpus.records[0].parent_id.as_deref(),
            Some("TOK_absent0000")
        );
        assert_eq!(corpus.records[0].end, None);
    }

    #[test]
    fn malformed_lines_fail_with_their_line_number() {
        let corpus: Vec<u8> = format!("{}\n{{not json}}\n", compact(&record_json())).into_bytes();
        assert!(matches!(
            Corpus::from_jsonl(&corpus),
            Err(CorpusError::Malformed { line: 2, .. })
        ));

        // A type error three records deep names the exact record and member.
        let corpus: Vec<u8> = format!(
            "{}\n{}\n{}\n",
            compact(&record_json()),
            compact(&record_json()),
            compact(&record_json().replace(r#""status": "ok""#, r#""status": 7"#))
        )
        .into_bytes();
        assert_eq!(
            Corpus::from_jsonl(&corpus),
            Err(CorpusError::InvalidField {
                line: 3,
                path: "record.status".to_owned(),
                detail: "expected a JSON string".to_owned()
            })
        );

        assert_eq!(Corpus::from_jsonl(b"\n  \n"), Err(CorpusError::Empty));
    }

    #[test]
    fn timestamps_are_validated_against_the_calendar() {
        for (value, detail) in [
            ("2026-13-01T00:00:00Z", "month is outside 01..=12"),
            ("2026-02-30T00:00:00Z", "day is outside the month's range"),
            ("2026-08-01T24:00:00Z", "hour is outside 00..=23"),
            ("2026-08-01T00:60:00Z", "minute is outside 00..=59"),
            ("2026-08-01T00:00:60Z", "second is outside 00..=59"),
            (
                "2026-08-01T00:00:00.1234567890Z",
                "fractional precision beyond nanoseconds is not representable",
            ),
            (
                "2026-08-01T00:00:00+01:00",
                "expected YYYY-MM-DDTHH:MM:SS[.fraction]Z",
            ),
            (
                "2026-08-01 00:00:00Z",
                "expected YYYY-MM-DDTHH:MM:SS[.fraction]Z",
            ),
        ] {
            let record = record_json().replace(
                r#""start": "2026-08-01T14:03:22.104Z""#,
                &format!(r#""start": "{value}""#),
            );
            assert_eq!(
                Corpus::from_jsonl(&corpus_with(&record)),
                Err(CorpusError::InvalidField {
                    line: 1,
                    path: "record.start".to_owned(),
                    detail: detail.to_owned()
                }),
                "{value} must be rejected"
            );
        }

        // Whole-second precision is retained as its own resolution, and leap
        // years are handled: 2024-02-29 is a real day, 2023-02-29 is not.
        let record = record_json().replace(
            r#""start": "2026-08-01T14:03:22.104Z""#,
            r#""start": "2024-02-29T00:00:00Z""#,
        );
        let corpus = Corpus::from_jsonl(&corpus_with(&record)).expect("leap day decodes");
        // A whole-second stamp carries one-second resolution, honestly.
        assert_eq!(
            corpus.records[0].start.resolution_nanoseconds,
            1_000_000_000
        );

        let record = record_json().replace(
            r#""start": "2026-08-01T14:03:22.104Z""#,
            r#""start": "2023-02-29T00:00:00Z""#,
        );
        assert!(Corpus::from_jsonl(&corpus_with(&record)).is_err());
    }

    #[test]
    fn error_detail_belongs_to_error_status() {
        let record = record_json().replace(
            r#""error": null"#,
            r#""error": { "kind": "timeout", "message": "TOK_1" }"#,
        );
        assert_eq!(
            Corpus::from_jsonl(&corpus_with(&record)),
            Err(CorpusError::InvalidField {
                line: 1,
                path: "record.error".to_owned(),
                detail: "error detail is present but status is not \"error\"".to_owned()
            })
        );

        let record = record_json()
            .replace(r#""status": "ok""#, r#""status": "error""#)
            .replace(
                r#""error": null"#,
                r#""error": { "kind": "timeout", "message": "TOK_1" }"#,
            );
        let corpus = Corpus::from_jsonl(&corpus_with(&record)).expect("error record decodes");
        assert_eq!(
            corpus.records[0]
                .error
                .as_ref()
                .map(|error| error.kind.as_str()),
            Some("timeout")
        );
    }

    /// Multi-byte UTF-8 inside a timestamp must be a structural error, never
    /// a slicing panic. Regression test for the byte-indexed parser rewrite.
    #[test]
    fn multibyte_characters_in_timestamps_fail_structurally_never_panic() {
        for value in [
            "2026-08-01T00:00:0\u{e9}Z",
            "2026-08-01T00:00:00.\u{e9}Z",
            "2026-08-01T00:00:00.10\u{e9}Z",
            "202\u{e9}-08-01T00:00:00Z",
            "2026-08-01T00:00:00Z\u{e9}",
        ] {
            let record = record_json().replace(
                r#""start": "2026-08-01T14:03:22.104Z""#,
                &format!(r#""start": "{value}""#),
            );
            assert!(
                Corpus::from_jsonl(&corpus_with(&record)).is_err(),
                "{value} must be rejected structurally"
            );
        }
    }

    /// Free text may be empty: SCHEMA.md types `error.message` and
    /// `labels[].comment` as plain strings, and the input value space is
    /// vendor-shaped. Identifiers and vocabulary stay non-empty — they become
    /// Event ids and kinds downstream, which the model rejects when empty,
    /// and a precise decode error beats an adaptation failure.
    #[test]
    fn empty_free_text_is_schema_valid_and_accepted() {
        let record = record_json()
            .replace(r#""status": "ok""#, r#""status": "error""#)
            .replace(
                r#""error": null"#,
                r#""error": { "kind": "timeout", "message": "" }"#,
            )
            .replace(
                r#""labels": [{ "key": "correctness", "value": "correct", "comment": null }]"#,
                r#""labels": [{ "key": "correctness", "value": "correct", "comment": "" }]"#,
            );
        let corpus = Corpus::from_jsonl(&corpus_with(&record)).expect("empty free text decodes");
        assert_eq!(
            corpus.records[0]
                .error
                .as_ref()
                .map(|error| error.message.as_str()),
            Some("")
        );
        assert_eq!(corpus.records[0].labels[0].comment.as_deref(), Some(""));

        // Identifiers stay non-empty: an empty id would produce a meaningless
        // Event identity downstream, so it is a decode error here.
        let record = record_json().replace(r#""id": "TOK_4f19a2e8c3""#, r#""id": """#);
        assert!(Corpus::from_jsonl(&corpus_with(&record)).is_err());
    }

    /// Empty keys are allowed in free values (`inputs`, `outputs`, label
    /// values) — arbitrary vendor JSON is carried verbatim — and rejected in
    /// the keyed maps the policy language addresses (`attributes`,
    /// `unmapped`, `links[].attributes`), where a dotted path with an empty
    /// segment is unaddressable and would be silently unmatched by any
    /// policy rule.
    #[test]
    fn empty_keys_follow_the_free_value_versus_keyed_map_line() {
        let record = record_json().replace(
            r#""inputs": { "user_id": "TOK_a3d17c9e02", "session_token": "TOK_6e5b28f4a1" }"#,
            r#""inputs": { "": 1, "user_id": "TOK_a3d17c9e02" }"#,
        );
        let corpus =
            Corpus::from_jsonl(&corpus_with(&record)).expect("empty key in inputs decodes");
        assert!(matches!(
            &corpus.records[0].inputs,
            CorpusValue::Object(entries) if entries.contains_key("")
        ));

        let record = record_json().replace(
            r#""attributes": { "service.name": "sagashop-payment" }"#,
            r#""attributes": { "": true, "service.name": "sagashop-payment" }"#,
        );
        assert!(Corpus::from_jsonl(&corpus_with(&record)).is_err());
    }

    /// A valid record followed by an invalid-UTF-8 line reports the second
    /// line's number, not zero: the error contract makes bad lines findable.
    #[test]
    fn invalid_utf8_is_reported_with_its_line_number() {
        let mut corpus = corpus_with(&compact(&record_json()));
        corpus.extend_from_slice(b"{\xff}\n");
        assert!(matches!(
            Corpus::from_jsonl(&corpus),
            Err(CorpusError::Malformed { line: 2, .. })
        ));
    }

    #[test]
    fn timestamp_epoch_and_precision_agree_with_known_values() {
        let record = record_json().replace(
            r#""start": "2026-08-01T14:03:22.104Z""#,
            r#""start": "1970-01-01T00:00:00Z""#,
        );
        let corpus = Corpus::from_jsonl(&corpus_with(&record)).expect("epoch decodes");
        assert_eq!(corpus.records[0].start.total_nanoseconds(), 0);
        assert_eq!(
            corpus.records[0].start.resolution_nanoseconds,
            1_000_000_000
        );

        let record = record_json().replace(
            r#""start": "2026-08-01T14:03:22.104Z""#,
            r#""start": "1970-01-01T00:00:00.5Z""#,
        );
        let corpus = Corpus::from_jsonl(&corpus_with(&record)).expect("half second decodes");
        assert_eq!(corpus.records[0].start.total_nanoseconds(), 500_000_000);
        assert_eq!(corpus.records[0].start.resolution_nanoseconds, 100_000_000);
    }
}
