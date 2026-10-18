//! Gemini spec drafting, validated against Sentinel's Pattern catalogue.
//!
//! A policy sentence is drafted by Gemini into one catalogue Pattern. The
//! draft is checked against the catalogue first. Jev then judges whether the
//! draft matches the sentence (`Same`, `Different`, or `Unknown`, using
//! `ledger-jev`'s fixed thresholds). Only a catalogue-valid draft that Jev
//! calls `Same` becomes a running monitor. Anything else is rejected with a
//! reason code.

mod author;
mod check;
mod draft;
mod monitor;
mod reason;

pub use author::{author, AuthoringInput, AuthoringOutcome, AuthoringStep, DraftFailure};
pub use check::{match_request, DRAFT_MATCH_QUESTION};
pub use draft::{
    draft_from_interaction, draft_request, DraftValue, PatternDraft, FIXTURE_POLICY, GEMINI_MODEL,
};
pub use monitor::{MonitorError, RunningMonitor};
pub use reason::{CatalogueCode, RejectReason};
