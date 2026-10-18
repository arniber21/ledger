//! OTel span ingest, normalised into canonical Sentinel events.
//!
//! One process subscriber fans spans to Sentry and to this crate. The ingest
//! path does not sample: every span the layer receives is normalised and
//! stored in the Timescale `events` hypertable.
//!
//! Spans that are ledger events (`agent.pay_bill`, and the exchange settlement
//! kinds) go through [`ledger_core::adapt_trace`]. `agent.decide` and the
//! exchange order spans are stored as well. A time-window count uses
//! `time_bucket` via [`ledger_store::Store::window_counts`].

mod capture;
mod error;
mod export;
mod normalize;
mod persist;

pub use capture::{CapturedSpan, IngestLayer, SpanSink};
pub use error::IngestError;
pub use export::IngestWorker;
pub use normalize::normalize;
pub use persist::persist;
