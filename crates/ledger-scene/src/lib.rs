//! Deterministic demo scenes.
//!
//! Each scene starts from its own rows, uses [`clock::epoch`], and reads Jev
//! from `fixtures/jev/` unless `LEDGER_JEV=live`. A missing replay fixture
//! fails. Replay does not call out. Nothing here records a Sentry incident.

mod clock;
mod play;
mod span;

pub use play::{run_all, run_scene, MonitorSnap, SceneReport, VerdictSnap, SCENES};

use ledger_jev::JevCallFailure;
use thiserror::Error;

/// Why a scene could not finish.
#[derive(Debug, Error)]
pub enum SceneError {
    /// The name is not one of [`SCENES`](crate::SCENES).
    #[error("unknown scene {0}")]
    Unknown(String),

    /// `LEDGER_JEV` was not `live` or `replay`.
    #[error("LEDGER_JEV must be live or replay")]
    JevMode,

    /// `LEDGER_TEST=1` and `LEDGER_JEV_FORCE` was not Same, Different, Unknown, or timeout.
    #[error("LEDGER_JEV_FORCE must be Same, Different, Unknown, or timeout")]
    Force,

    /// `LEDGER_JEV=live` and `TYPESAFE_API_KEY` is unset. The value is not printed.
    #[error("TYPESAFE_API_KEY is unset")]
    MissingJevKey,

    /// The scene's expected verdicts were not what the stack produced.
    #[error("{0}")]
    Check(String),

    /// Replay had no saved body for this request.
    #[error(transparent)]
    Replay(#[from] ledger_jev::ReplayError),

    /// Timescale rejected a query.
    #[error(transparent)]
    Store(#[from] ledger_store::StoreError),

    /// Ingest could not store a span.
    #[error(transparent)]
    Ingest(#[from] ledger_ingest::IngestError),

    /// The verdict engine could not seal.
    #[error(transparent)]
    Api(#[from] ledger_api::ApiError),

    /// A ledger event could not be built.
    #[error(transparent)]
    Adapt(#[from] ledger_core::AdapterError),

    /// A live Jev call failed. Replay does not use this path.
    #[error(transparent)]
    Jev(#[from] JevCallFailure),
}
