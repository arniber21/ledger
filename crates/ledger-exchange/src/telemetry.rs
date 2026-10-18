//! Process-wide Sentry tracing and logs for the exchange.
//!
//! Call [`install_telemetry`] once from the process entrypoint before any
//! order flow. Order-book methods only emit `tracing` spans; they do not
//! install a subscriber.
//!
//! The same subscriber fans every span to Sentry and to ledger-ingest.
//! Ingest has no sample rate. Sentry stays at `traces_sample_rate` 1.0 with
//! `send_default_pii` false.
//!
//! Feature names were checked against the Sentry Rust docs for SDK 0.49
//! (`https://docs.sentry.io/platforms/rust/logs/`):
//! - `tracing` enables the `sentry-tracing` integration at
//!   `sentry::integrations::tracing`.
//! - `logs` enables structured logs. It is also a default feature; it is named
//!   here so a build without default features still sends logs.

use ledger_ingest::{IngestError, IngestWorker};
use sentry::integrations::tracing::EventFilter;
use std::time::Duration;
use thiserror::Error;
use tracing_subscriber::prelude::*;

/// Guard that flushes ingest and Sentry when dropped.
#[must_use = "dropping the guard flushes ingest and Sentry and ends telemetry"]
pub struct TelemetryGuard {
    worker: Option<IngestWorker>,
    _client: sentry::ClientInitGuard,
}

/// Why exchange telemetry could not start.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum TelemetryError {
    /// `SENTRY_DSN` was missing or empty. The value is never included here.
    #[error("SENTRY_DSN is not set")]
    MissingDsn,
    /// The DSN was present but the SDK did not enable a client.
    #[error("Sentry client is disabled")]
    Disabled,
    /// Another subscriber is already installed in this process.
    #[error("tracing subscriber is already installed")]
    AlreadyInstalled,
    /// Timescale ingest could not start or could not store a span.
    #[error("ingest failed: {0}")]
    Ingest(#[source] IngestError),
}

impl From<IngestError> for TelemetryError {
    fn from(error: IngestError) -> Self {
        Self::Ingest(error)
    }
}

/// Starts Sentry from `SENTRY_DSN` and forwards `tracing` spans and logs.
///
/// Sample rate is 1.0 so every order trace is sent. `send_default_pii` is
/// false. The same subscriber forwards every span at info or above to
/// ledger-ingest, which does not sample. Error-level events are captured as
/// Sentry errors and as logs. Info and warn events, including order
/// rejections, are logs and breadcrumbs, not error events.
///
/// # Errors
///
/// Returns [`TelemetryError::MissingDsn`] when `SENTRY_DSN` is unset or blank,
/// [`TelemetryError::Disabled`] when the SDK rejects the DSN, and
/// [`TelemetryError::AlreadyInstalled`] when a tracing subscriber already exists.
pub fn install_telemetry() -> Result<TelemetryGuard, TelemetryError> {
    if !dsn_is_set() {
        return Err(TelemetryError::MissingDsn);
    }

    let client = sentry::init(
        sentry::ClientOptions::new()
            .maybe_release(sentry::release_name!())
            .send_default_pii(false)
            .traces_sample_rate(1.0)
            .shutdown_timeout(Duration::from_secs(5)),
    );

    let enabled = sentry::Hub::current()
        .client()
        .is_some_and(|active| active.is_enabled());
    if !enabled {
        return Err(TelemetryError::Disabled);
    }

    let (ingest_layer, worker) = IngestWorker::start()?;
    let sentry_layer = sentry::integrations::tracing::layer().event_filter(event_filter);

    let filter = tracing_subscriber::filter::LevelFilter::INFO;
    // The level gate is not a sampler. Ingest and Sentry both receive every
    // span at info or above. Sentry's trace sample rate stays 1.0.
    let installed = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter))
        .with(sentry_layer.with_filter(filter))
        .with(ingest_layer.with_filter(filter))
        .try_init();
    if installed.is_err() {
        drop(worker);
        return Err(TelemetryError::AlreadyInstalled);
    }

    Ok(TelemetryGuard {
        worker: Some(worker),
        _client: client,
    })
}

impl TelemetryGuard {
    /// Writes queued spans to Timescale, then flushes Sentry when dropped.
    ///
    /// # Errors
    ///
    /// Returns [`TelemetryError::Ingest`] when a queued span could not be stored.
    pub fn close(mut self) -> Result<(), TelemetryError> {
        match self.worker.take() {
            Some(worker) => worker.close().map_err(TelemetryError::from),
            None => Ok(()),
        }
    }
}

/// Maps tracing levels onto Sentry events, logs, and breadcrumbs.
///
/// Error events, including a rejected fill, are Sentry errors. Info and warn
/// events are logs and breadcrumbs.
pub(crate) fn event_filter(metadata: &tracing::Metadata<'_>) -> EventFilter {
    match *metadata.level() {
        tracing::Level::ERROR => EventFilter::Event | EventFilter::Log,
        tracing::Level::DEBUG | tracing::Level::TRACE => EventFilter::Ignore,
        tracing::Level::INFO | tracing::Level::WARN => EventFilter::Breadcrumb | EventFilter::Log,
    }
}

/// True when `SENTRY_DSN` is set to a non-empty value. The value is not logged.
fn dsn_is_set() -> bool {
    std::env::var("SENTRY_DSN")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
}
