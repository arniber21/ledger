//! Write normalised spans into the events hypertable.

use ledger_store::{Event, Store};

use crate::capture::CapturedSpan;
use crate::error::IngestError;
use crate::normalize::normalize;

/// Normalises `spans` and inserts one hypertable row per span.
///
/// # Errors
///
/// Returns [`IngestError::Store`] when an insert fails. Spans inserted before
/// the failure stay stored.
pub async fn persist(store: &Store, spans: &[CapturedSpan]) -> Result<Vec<Event>, IngestError> {
    let events = normalize(spans);
    for event in &events {
        store.insert_event(event).await?;
    }
    Ok(events)
}
