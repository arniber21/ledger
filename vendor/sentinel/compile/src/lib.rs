//! Specification compiler and Pattern catalogue.
//!
//! Compiles Patterns (ADR-0008) into monitor IR for the Verification Core,
//! and emits the attribute-level Event Filter (ADR-0006, O-08) that Probes
//! push down. Routes work to the hot or cold tier by Pattern semantics
//! (ADR-0020).
//!
//! [`catalogue`] holds the Patterns themselves, declared as data. Read its
//! module docs first: they state what grounds the catalogue, and the answer is
//! thinner than "partner-derived" suggests.
//!
//! [`subsumption()`] asks whether a Candidate can add any Violate a deployed
//! Specification does not already reach (issue #177, ADR-0037). It can prove
//! containment and never refutes it; its module docs say why.

pub mod catalogue;
mod filter;
pub mod ir;
mod subsumption;
mod width;

pub use filter::{
    activation_watermarks_for, compile_filter, ActivationStamp, CompileError, CompiledFilter,
    EventDependency, FilterIdentity, FilterSource,
};
// `SelectorDivergence` and `RetirementIr` are re-exported at the crate root as
// well as through `pub mod ir`, because both are carried inside `Undecided` and
// the consumer of this analysis is `candidate/`, which
// `check-candidate-grammar.sh` clause C1 forbids from naming the path
// `sentinel_compile::ir` at all.
pub use ir::{RetirementIr, SelectorDivergence};
pub use subsumption::{
    subsumption, CandidateUnderReview, DeployedSpecification, EvidenceInclusion,
    InclusionDirection, NotComparable, SelectorInclusion, Subsumption, SubsumptionProof, Undecided,
};
pub use width::{aggregate_by_pattern, FilterWidthBudget, PatternWidthReport, WidthReport};
