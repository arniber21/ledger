//! Process-wide Sentry tracing and logs for the exchange.
//!
//! Call [`install_telemetry`] once from the process entrypoint before any
//! order flow. Order-book methods only emit `tracing` spans; they do not
//! install a subscriber.
//!
//! Feature names were checked against the Sentry Rust docs for SDK 0.49
//! (`https://docs.sentry.io/platforms/rust/logs/`):
//! - `tracing` enables the `sentry-tracing` integration at
//!   `sentry::integrations::tracing`.
//! - `logs` enables structured logs. It is also a default feature; it is named
//!   here so a build without default features still sends logs.

use sentry::integrations::tracing::EventFilter;
use std::time::Duration;
use thiserror::Error;
use tracing_subscriber::prelude::*;

/// Guard that flushes Sentry when dropped.
#[must_use = "dropping the guard flushes Sentry and ends telemetry"]
pub struct TelemetryGuard {
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
}

/// Starts Sentry from `SENTRY_DSN` and forwards `tracing` spans and logs.
///
/// Sample rate is 1.0 so every order trace is sent. `send_default_pii` is
/// false. Error-level events are captured as Sentry errors and as logs.
/// Info and warn events, including order rejections, are logs and breadcrumbs,
/// not error events.
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

    let sentry_layer = sentry::integrations::tracing::layer().event_filter(event_filter);

    let filter = tracing_subscriber::filter::LevelFilter::INFO;
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter))
        .with(sentry_layer.with_filter(filter))
        .try_init()
        .map_err(|_| TelemetryError::AlreadyInstalled)?;

    Ok(TelemetryGuard { _client: client })
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
