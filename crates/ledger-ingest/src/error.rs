//! Failures while starting ingest or writing events.

use ledger_store::StoreError;
use thiserror::Error;

/// Why ingest could not start or could not store a span.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum IngestError {
    /// The background worker exited before it acknowledged a command.
    #[error("ingest worker stopped")]
    WorkerStopped,

    /// The worker thread or its runtime could not be created.
    #[error("ingest worker failed to start: {0}")]
    Startup(String),

    /// Timescale rejected a connection or an insert.
    #[error(transparent)]
    Store(#[from] StoreError),
}
