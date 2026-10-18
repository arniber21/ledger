//! Failures while adapting a ledger trace or sealing a Sentinel verdict.

use sentinel_compile::catalogue::InstanceError;
use sentinel_core::specification::SpecificationConversionError;
use sentinel_core::CoreError;
use sentinel_model::{CompletenessSignalError, EventError, IdentifierError};
use thiserror::Error;

/// Why a ledger trace could not be adapted or judged.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AdapterError {
    /// An identifier was empty or whitespace.
    #[error("blank {0}")]
    Blank(&'static str),
    /// Two ledger events in one trace used the same id.
    #[error("duplicate ledger event id {0}")]
    DuplicateEvent(String),
    /// A causal predecessor was blank, unknown, or not yet in the trace.
    #[error("event {event} names unknown predecessor {predecessor}")]
    UnknownPredecessor {
        /// The event that named the predecessor.
        event: String,
        /// The predecessor that was not an earlier event in the trace.
        predecessor: String,
    },
    /// An event listed its own id as a causal predecessor.
    #[error("event {event} lists itself as a predecessor")]
    SelfPredecessor {
        /// The event id.
        event: String,
    },
    /// A per-source sequence counter overflowed `u64`.
    #[error("sequence number overflowed for a source")]
    SequenceOverflow,
    /// Catalogue instantiation did not lower to the absence monitor.
    #[error("catalogue instantiation did not lower to an absence monitor")]
    NotAbsence,
    /// A Sentinel event failed validation.
    #[error(transparent)]
    Event(#[from] EventError),
    /// A completeness signal failed validation.
    #[error(transparent)]
    Completeness(#[from] CompletenessSignalError),
    /// A Sentinel identifier was rejected.
    #[error(transparent)]
    Identifier(#[from] IdentifierError),
    /// The absence Pattern could not be instantiated.
    #[error(transparent)]
    Spec(#[from] InstanceError),
    /// Lowered absence IR could not become a Core specification.
    #[error(transparent)]
    Specification(#[from] SpecificationConversionError),
    /// The incremental evaluator refused the trace.
    #[error(transparent)]
    Core(#[from] CoreError),
}
