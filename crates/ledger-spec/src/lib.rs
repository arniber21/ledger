//! Gemini spec drafting, validated against Sentinel's Pattern catalogue.
//!
//! A policy sentence is drafted by Gemini into one catalogue Pattern. The
//! draft is checked against the catalogue first. Jev then judges whether the
//! draft matches the sentence (`Same`, `Different`, or `Unknown`, using
//! `ledger-jev`'s fixed thresholds). Code splits the sentence into clauses
//! and Jev judges each clause in one batched Choice request. A catalogue-valid
//! draft that Jev calls `Same`, with every clause covered, becomes a running
//! monitor. An uncovered or Unknown clause holds arming unless the author
//! overrides. Anything else is rejected with a reason code.

mod author;
mod check;
mod clause;
mod draft;
mod monitor;
mod reason;
mod replay_author;

pub use author::{author, AuthoringInput, AuthoringOutcome, AuthoringStep, DraftFailure};
pub use check::{match_request, DRAFT_MATCH_QUESTION};
pub use clause::{
    clause_question_id, coverage_request, hold_reason, judge_coverage, split_clauses,
    ClauseCoverage, HoldReason, MarkedClause,
};
pub use draft::{
    draft_from_interaction, draft_request, DraftValue, PatternDraft, FIXTURE_POLICY, GEMINI_MODEL,
};
pub use monitor::{MonitorError, RunningMonitor};
pub use reason::{CatalogueCode, RejectReason};
pub use replay_author::{author_replay, CATALOGUE_REJECT_POLICY, TWO_CLAUSE_POLICY};
