//! Process-wide Sentry tracing and logs for the exchange.
//!
//! Call [`install_telemetry`] once from the process entrypoint before any
//! order flow. Order-book methods only emit `tracing` spans; they do not
//! install a subscriber.
//!
//! The same subscriber fans every span to ledger-ingest, and to Sentry when
//! `SENTRY_DSN` is set. Ingest has no sample rate. Sentry stays at
//! `traces_sample_rate` 1.0 with `send_default_pii` false.
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
    _client: Option<sentry::ClientInitGuard>,
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

/// Forwards `tracing` spans to ledger-ingest, and to Sentry when `SENTRY_DSN` is set.
///
/// Ingest starts even when `SENTRY_DSN` is unset, so `agent.decide`,
/// `agent.pay_bill`, and the exchange match, fill, and settle spans are still
/// stored. Sentry's sample rate is 1.0 when the DSN is set. `send_default_pii`
/// is false. Ingest does not sample. Error-level events are captured as Sentry
/// errors and as logs. Info and warn events, including order rejections, are
/// logs and breadcrumbs, not error events.
///
/// # Errors
///
/// Returns [`TelemetryError::Disabled`] when `SENTRY_DSN` is set and the SDK
/// rejects it, [`TelemetryError::Ingest`] when that same process cannot store
/// spans, and [`TelemetryError::AlreadyInstalled`] when a tracing subscriber
/// already exists. A missing DSN does not fail startup: ingest is attempted,
/// and a fmt subscriber is installed when Timescale cannot be reached.
pub fn install_telemetry() -> Result<TelemetryGuard, TelemetryError> {
    let client = start_sentry()?;
    let ingest = IngestWorker::start();
    let (ingest_layer, worker) = match ingest {
        Ok((layer, worker)) => (Some(layer), Some(worker)),
        Err(error) if client.is_some() => return Err(error.into()),
        Err(_) => (None, None),
    };

    let filter = tracing_subscriber::filter::LevelFilter::INFO;
    // The level gate is not a sampler. Ingest receives every span at info or
    // above. Sentry's trace sample rate stays 1.0 when the client is on.
    let installed = match (client.is_some(), ingest_layer) {
        (true, Some(ingest_layer)) => {
            let sentry_layer = sentry::integrations::tracing::layer().event_filter(event_filter);
            tracing_subscriber::registry()
                .with(tracing_subscriber::fmt::layer().with_filter(filter))
                .with(sentry_layer.with_filter(filter))
                .with(ingest_layer.with_filter(filter))
                .try_init()
        }
        (false, Some(ingest_layer)) => tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_filter(filter))
            .with(ingest_layer.with_filter(filter))
            .try_init(),
        (_, None) => tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_filter(filter))
            .try_init(),
    };
    if installed.is_err() {
        drop(worker);
        return Err(TelemetryError::AlreadyInstalled);
    }

    Ok(TelemetryGuard {
        worker,
        _client: client,
    })
}

/// Starts Sentry when `SENTRY_DSN` is set. `Ok(None)` means Sentry is off and
/// ingest should still run.
fn start_sentry() -> Result<Option<sentry::ClientInitGuard>, TelemetryError> {
    if !dsn_is_set() {
        return Ok(None);
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
    Ok(Some(client))
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
