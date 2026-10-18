//! Rejection reasons for spec authoring.
//!
//! Reasons are codes. The model is not asked for explanation text, and none of
//! these variants carry model prose.

use serde_json::{json, Value};

/// Why a draft was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RejectReason {
    /// The policy sentence was empty.
    EmptyPolicy,
    /// Gemini timed out or the call failed before a body arrived.
    DraftUnavailable,
    /// The Gemini body was not a completed draft object.
    DraftUnreadable,
    /// `pattern_id` is not in the Sentinel catalogue.
    UnknownPattern,
    /// The draft names a catalogue Pattern but does not bind it.
    CatalogueInvalid {
        /// Which catalogue check failed.
        code: CatalogueCode,
    },
    /// Jev judged the draft a different property from the sentence.
    DraftDiffers,
    /// Jev timed out, erred, scored below the threshold, or contradicted itself.
    ///
    /// This is not an acceptance.
    MatchInconclusive,
    /// The catalogue accepted the parameters but the monitor could not start.
    MonitorFailed,
}

/// A deterministic catalogue check that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CatalogueCode {
    /// A declared parameter was missing.
    MissingParameter {
        /// Parameter name.
        name: String,
    },
    /// The draft bound a parameter the Pattern does not declare.
    UnknownParameter {
        /// Parameter name.
        name: String,
    },
    /// A parameter value had the wrong kind.
    KindMismatch {
        /// Parameter name.
        name: String,
    },
    /// A selector named an event kind the ledger does not emit.
    EventKindNotEmitted {
        /// Parameter name.
        name: String,
        /// The event kind the draft named.
        event_kind: String,
    },
    /// An attribute-key parameter was not the bill id the ledger emits.
    BindingAttributeNotEmitted {
        /// Parameter name.
        name: String,
    },
}

impl RejectReason {
    pub(crate) fn sse_data(&self) -> Value {
        match self {
            Self::EmptyPolicy => json!({ "reason": "empty_policy" }),
            Self::DraftUnavailable => json!({ "reason": "draft_unavailable" }),
            Self::DraftUnreadable => json!({ "reason": "draft_unreadable" }),
            Self::UnknownPattern => json!({ "reason": "unknown_pattern" }),
            Self::DraftDiffers => json!({ "reason": "draft_differs" }),
            Self::MatchInconclusive => json!({ "reason": "match_inconclusive" }),
            Self::MonitorFailed => json!({ "reason": "monitor_failed" }),
            Self::CatalogueInvalid { code } => code.sse_data(),
        }
    }
}

impl CatalogueCode {
    fn sse_data(&self) -> Value {
        match self {
            Self::MissingParameter { name } => json!({
                "reason": "catalogue_invalid",
                "code": "missing_parameter",
                "parameter": name
            }),
            Self::UnknownParameter { name } => json!({
                "reason": "catalogue_invalid",
                "code": "unknown_parameter",
                "parameter": name
            }),
            Self::KindMismatch { name } => json!({
                "reason": "catalogue_invalid",
                "code": "kind_mismatch",
                "parameter": name
            }),
            Self::EventKindNotEmitted { name, event_kind } => json!({
                "reason": "catalogue_invalid",
                "code": "event_kind_not_emitted",
                "parameter": name,
                "event_kind": event_kind
            }),
            Self::BindingAttributeNotEmitted { name } => json!({
                "reason": "catalogue_invalid",
                "code": "binding_attribute_not_emitted",
                "parameter": name
            }),
        }
    }
}
