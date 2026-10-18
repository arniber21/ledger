//! Ledger domain types and the adapter from ledger events to Sentinel `Event`s.
//!
//! The exchange is one source. Each agent's tool-call log is another. Precedence,
//! absence-after, and response monitors are judged with Sentinel's incremental
//! `Evaluator::step` + `seal` path.

mod adapt;
mod error;
mod event;
mod judge;
mod live;
mod source;
mod specs;

pub mod dataset;

pub use adapt::{adapt_trace, AdaptedTrace, TraceCursor, WithheldSource};
pub use error::AdapterError;
pub use event::{AgentId, BillId, LedgerEvent, LedgerKind, OrderEnd, OrderId, Times};
pub use judge::judge_bill_absence;
pub use live::{Judged, Session, TimelineEntry, RESPONSE_DEADLINE_TICKS};
pub use sentinel_core::verdict::BindingVerdict;
pub use sentinel_model::{
    DataClass, InconclusiveReason, Integrity, SourceCapability, SourceTier, VerdictAuthority,
    VerdictOutcome,
};
pub use source::{
    agent_source, capabilities, event_kind, exchange_source, schema_version, tier_of,
    AGENT_ATTRIBUTE, BILL_ATTRIBUTE, ORDER_ATTRIBUTE,
};
pub use specs::{SPEC_ABSENCE, SPEC_PRECEDENCE, SPEC_RESPONSE};
