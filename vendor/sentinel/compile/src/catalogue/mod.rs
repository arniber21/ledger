//! The Pattern catalogue: ADR-0008's frontend and generation grammar, as data.
//!
//! # What grounds this catalogue, stated before anything else
//!
//! **No design partner has supplied a policy, an incident, or a trace.** Issue
//! #1 closed on an explicit maintainer decision to proceed on internal
//! validation, and `docs/research/m0-evidence-corpus.md` says of its own five
//! scenarios: *"No scenario in this corpus is backed by a captured Trace. They
//! are validated designs for properties, not measured evidence."* Every
//! [`GroundingRef`] below therefore resolves to an internal scenario, a
//! deterministic testbed, or an ADR — never to a partner policy. That is why
//! [`EvidenceGrade::PartnerEvidenced`] exists and why no Pattern here carries
//! it; `no_first_catalogue_pattern_claims_partner_evidence` pins that in a test
//! rather than in this sentence.
//!
//! Thin is not the same as absent, and overstating the thinness fails the same
//! honesty discipline as overstating the grounding. The corpus is explicit that
//! it *"evidences four (P-A through P-D) — three of them
//! precedence/approval-shaped"*; only P-E (provenance/taint) is a gap. CAT-1 is
//! P-A.
//!
//! CAT-4 **is** P-E, and it is therefore the one entry here with no scenario
//! behind it at all. It carries [`EvidenceGrade::Unevidenced`], which is not a
//! lesser citation but a different kind of claim: the corpus states in terms
//! that #14 "must either author P-E directly against ADR-0016's example —
//! declared as unevidenced — or wait for partner data", and this is the first
//! branch taken deliberately. Its grounding references resolve to ADR-0016,
//! ADR-0020, ADR-0022 and ADR-0025, and to the corpus's own sentence recording
//! the gap — never to a scenario. In particular the corpus's instruction *"Do
//! not present S1's lease lineage as taint evidence"* is obeyed: S1 appears in
//! CAT-1's grounding and nowhere in CAT-4's.
//!
//! # What "traceable" means here, mechanically
//!
//! O-07 asks catalogue entries to trace back to their origin. Prose cannot be
//! checked, so each [`GroundingRef`] carries a repo-relative `document` and an
//! `anchor` string that must occur **verbatim** in that document.
//! `every_grounding_reference_resolves` (in `compile/tests/catalogue_snapshot.rs`)
//! opens every document and searches for every anchor. A citation that rots is a
//! failing test, not a stale footnote.
//!
//! One consequence is load-bearing: the corpus's provenance ledger marks **only
//! S1** with the parenthetical `internal-scenario (expert-authored, no partner
//! trace)`; S2 through S5 read a bare `internal-scenario`. Quoting the long
//! string for S2-S5 would be a fabricated quotation inside the very structure
//! whose purpose is mechanical traceability. The refs below quote the corpus's
//! own summary line instead, which covers all five.
//!
//! # Why this is Rust data and not a `.proto`
//!
//! Neither issue #21 (generation) nor #35 (catalogue UI) is blocked on a wire
//! format; both are blocked on a **stable, enumerable parameter schema**, which
//! [`catalogue`] supplies today. `just gen-probes` is a networked maintenance
//! boundary the gates never invoke (ADR-0024), and ADR-0024's canonical list
//! does not include Patterns. The cheap substitute for a wire contract is the
//! golden snapshot at `compile/tests/golden/catalogue.txt`: any change to the
//! grammar is a reviewed diff. A `sentinel.catalogue.v1` proto is a named
//! follow-up.
//!
//! # Which variants ship, and the rule that decides
//!
//! #14a shipped the declaration layer plus CAT-1; issue #141 adds CAT-4 and
//! CAT-2; issue #173 adds CAT-5 `aggregation.count-within`; issue #210 adds
//! CAT-3 `existence.before-scope-close` with [`ScopeKind::Between`] and
//! [`RetirementPolicy::Deadline`]. So the enums here follow one rule, applied
//! uniformly:
//!
//! - A **unit variant transcribed from established vocabulary** costs nothing
//!   and invents nothing, so it ships. [`TemporalShape`]'s first five members
//!   are CONTEXT.md's definition of Pattern and ADR-0008's Dwyer list verbatim;
//!   [`EvaluationTier`]'s two are ADR-0005's tiers.
//! - A **variant carrying data is a design decision** — how a delimiter is
//!   named, how a deadline is represented — and waits for the Pattern that
//!   needs it. CAT-2's interval stays selector *parameters* (`after`, `until`,
//!   `veto`) on an [`ScopeKind::EventLocal`] declaration that closes at
//!   `until`. CAT-3 spends the Between reservation as a **unit** variant:
//!   open/required/close remain selector parameters, not ScopeKind payload.
//!
//!   The deadline half of that reservation is **answered** rather than spent
//!   again: a retirement policy is a *declaration*, the duration is a periphery
//!   parameter, and so [`RetirementPolicy::Deadline`] and
//!   [`RetirementPolicy::Inactivity`] ship as unit variants transcribed from
//!   `docs/PRD.md`'s O-03 wording. See [`RetirementPolicy`] for the argument
//!   and for the watermark-bounded policy that is dropped rather than deferred.
//!   CAT-3 declares Deadline only; Inactivity remains grammar-only (Core
//!   mechanism pin for hand-built fixtures) and is not this Pattern's
//!   declaration.
//! - A **unit variant that is not transcribed from anywhere** — the sixth
//!   [`TemporalShape`], `Reachability` — is the third case, and it ships only
//!   because the Pattern that needs it is here. #14a wrote down in advance that
//!   this shape is what CAT-4 needs and that it is not a Dwyer shape; adding it
//!   now is that reservation being spent, not a new invention. What it is *not*
//!   is a licence for an `EY`-class operator: see [`PatternId::ProvenanceTaint`].
//!   [`TemporalShape::Absence`] / [`TemporalShape::Existence`] are the opposite
//!   case: transcribed from day one, and spent by CAT-2 / CAT-3.
//!
//! Every enum is `#[non_exhaustive]`, so adding the rest is additive.
//!
//! # Where the catalogue stops, said before a reader assumes otherwise
//!
//! CAT-1, CAT-2, CAT-3, CAT-4 and CAT-5 are **declared, lowered, and evaluated
//! end-to-end.** `Evaluator` hides catalogued families behind a private
//! `Monitored` enum. `catalogue_declares_no_pattern_the_core_cannot_lower`
//! must stay exhaustive over every catalogued id.

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.
#![allow(clippy::module_name_repetitions)]

mod exercise;
mod instance;

pub use exercise::{
    CorpusExercise, CounterpartMatches, SelectorExercise, SettledEvidence, TriggerExercise,
};
pub use instance::{
    instantiate, EvidenceBinding, InstanceError, InstanceWarning, ParameterValue, PatternInstance,
    ProducerContract, SelectorValue,
};

use sentinel_model::{SourceCapability, SourceTier};

use crate::width::FilterWidthBudget;

/// Stable identity of one catalogued Pattern.
///
/// The string form is what a Candidate, a `SpecificationFilter` or a catalogue
/// API names; the enum is what an exhaustive `match` in [`instantiate`] keys on.
/// Adding a Pattern is a reviewed act (ADR-0008), which is what
/// `#[non_exhaustive]` and the golden snapshot together make true in practice.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum PatternId {
    /// CAT-1 `precedence.approval` — an Event never occurs unless a qualifying
    /// Event for the same Binding lies in its causal past.
    PrecedenceApproval,
    /// CAT-4 `provenance.taint` — an Event never occurs when an Event marking
    /// untrusted content for the same Binding lies in its causal past.
    ///
    /// The second flagship family (ADR-0008), and ADR-0016's own example of the
    /// differentiated property: *"an action for a particular recipient and
    /// workflow did not have the required causally prior approval, **or was
    /// influenced by untrusted content**"*. CAT-1 is the first clause; this is
    /// the second.
    ///
    /// # Why this is hot when ADR-0020 says the family is cold
    ///
    /// ADR-0020 costs the catalogue's fragment boundary as *"provenance / taint
    /// reachability is cold"*, and that is about **propagation**: "a path from a
    /// taint source with no sanitizer on it" is a since-like operator over the
    /// causal DAG, which is PaCTL and needs the cold path. ADR-0016's example
    /// asks for less. "Influenced by untrusted content" is
    /// `AG(action -> !EP(untrusted))` — the same past-time `EP` over the same
    /// causal ancestry CAT-1 already walks, in the opposite polarity — so it
    /// sits inside ADR-0020's PaBTL fragment, runs hot, and costs zero
    /// cold-path fact requests.
    ///
    /// That is not a convenience. ADR-0025 closes with *"no user-facing Pattern
    /// exposes one [an `EY`-class operator] until a partner property justifies
    /// it"*, O-20b says the same, and `core/src/facts.rs` records that inventing
    /// a reachability query early *"is exactly what ADR-0019 forbids"*. There is
    /// no partner property. A cold, `EY`-class CAT-4 would contradict three
    /// places at once; the propagating member of this family waits with O-20b,
    /// and CAT-4 is the non-propagating one.
    ProvenanceTaint,
    /// CAT-2 `absence.after-until` — a named Event never occurs inside the open
    /// interval `(after, until]` for the same Binding.
    ///
    /// The corpus's P-B ("no veto Event between approval and action"). The Scope
    /// closes at `until`, which is what licenses
    /// [`RetirementPolicy::NotRequired`] without inventing `ScopeKind::Between`:
    /// the delimiters are selector parameters, not Scope-enum payload. The
    /// census keys on `until` and never on `veto` — see
    /// [`crate::catalogue::exercise`] and [`crate::ir::AbsenceIr`].
    AbsenceAfterUntil,
    /// CAT-5 `aggregation.count-within` — at a trigger/assessment Event, the
    /// count of matching member Events inside a **sequence** lookback must stay
    /// strictly below a threshold for the same Binding.
    ///
    /// The first Dogwood-parity aggregation member (O-07 / #173 / #84). Lookback
    /// is sequence positions along causal Event order — **not** wall-clock
    /// `s/m/h/d`, and not Scope / Deadline. Cold, detect-only
    /// ([`EvaluationTier::Cold`]); the census keys on `trigger` and never on
    /// `member` — see [`crate::catalogue::exercise`] and
    /// [`crate::ir::AggregateIr`].
    AggregationCountWithin,
    /// CAT-3 `existence.before-scope-close` — a named Event occurs at least once
    /// inside a Between Scope before that Scope closes.
    ///
    /// Declares [`ScopeKind::Between`] and [`RetirementPolicy::Deadline`]. Open,
    /// required, and close are selector parameters; interval membership is
    /// `(open, close]`. Detect-only in this PR (ADR-0023 gates 4/5 unclaimed).
    ExistenceBeforeScopeClose,
}

impl PatternId {
    /// Returns the stable string identifier of this Pattern.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PrecedenceApproval => "precedence.approval",
            Self::ProvenanceTaint => "provenance.taint",
            Self::AbsenceAfterUntil => "absence.after-until",
            Self::AggregationCountWithin => "aggregation.count-within",
            Self::ExistenceBeforeScopeClose => "existence.before-scope-close",
        }
    }
}

impl std::fmt::Display for PatternId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The temporal shape a Pattern instantiates.
///
/// The **first five** are CONTEXT.md's definition of Pattern ("absence,
/// existence, universality, precedence, response") and ADR-0008's Dwyer list,
/// transcribed. The string forms are exactly `dashboard/src/data/mock.ts`'s
/// `PatternShape` union, so #35 renders the real catalogue without extending its
/// type — `shape_names_match_the_dashboard_pattern_shape_union` keeps the two
/// vocabularies from drifting.
///
/// [`TemporalShape::Reachability`] is the sixth and is **not** a Dwyer shape.
/// #14a wrote the reservation down rather than leaving it implicit — it named
/// reachability as "the shape CAT-4 provenance/taint needs" and left it out
/// until that Pattern existed. Issue #141 is that Pattern, so the reservation is
/// spent here. Read [`PatternId::ProvenanceTaint`] before concluding that the
/// name implies the cold path: it does not, and the reason is written there.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum TemporalShape {
    /// A named Event never occurs within the Scope.
    Absence,
    /// A named Event occurs at least once within the Scope.
    Existence,
    /// A condition holds at every Event within the Scope.
    Universality,
    /// A named Event never occurs unless another lies in its causal past.
    Precedence,
    /// A named Event is always eventually followed by another, causally.
    Response,
    /// A named Event never occurs when a prohibited Event can reach it.
    ///
    /// The property is a relation between two Events rather than the occurrence
    /// of one within an extent, which is what makes it a different shape from
    /// [`TemporalShape::Absence`] rather than a scoped case of it — and what
    /// makes it uncheckable by a local guardrail (ADR-0016). Its Violate rests
    /// on a **presence**, which inverts which row of a decision table is the
    /// default; `core/src/taint.rs` carries what that costs.
    Reachability,
    /// A numeric aggregate over member Events inside a lookback is bounded.
    ///
    /// Not a Dwyer shape. Dogwood's `count_within` / `sum_within` /
    /// `count_distinct_within` family (O-07 / #173 / #84). Lookback is a Pattern
    /// parameter (sequence positions), never Scope and never wall-clock Duration
    /// silently lowered.
    Aggregation,
}

impl TemporalShape {
    /// Every shape: CONTEXT.md's and ADR-0008's five in their order, then the
    /// shapes this catalogue adds.
    ///
    /// Enumerable because #21 classifies a Candidate into one of these and #35
    /// renders one card per shape; both need the list, not just the type.
    pub const ALL: [Self; 7] = [
        Self::Absence,
        Self::Existence,
        Self::Universality,
        Self::Precedence,
        Self::Response,
        Self::Reachability,
        Self::Aggregation,
    ];

    /// Returns the shape's identifier, matching `PatternShape` in the dashboard.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Absence => "absence",
            Self::Existence => "existence",
            Self::Universality => "universality",
            Self::Precedence => "precedence",
            Self::Response => "response",
            Self::Reachability => "reachability",
            Self::Aggregation => "aggregation",
        }
    }
}

/// The extent over which a Pattern is evaluated (CONTEXT.md, *Scope*).
///
/// ADR-0008 also names before, after and between scopes. They are absent here
/// because each needs a declaration-level representation of its delimiting
/// Events, and that representation is a decision belonging to the first Pattern
/// that has delimiters — see the module docs.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ScopeKind {
    /// The Scope is one trigger Event and its causal past, opening and closing
    /// at that Event.
    ///
    /// This is the corpus's term (`m0-evidence-corpus.md`, S5: *"per request
    /// (Event-local — the trigger's causal past)"*), and it is what licenses
    /// [`RetirementPolicy::NotRequired`]: a Scope that closes at the Event that
    /// opened it cannot fail to close, so there is nothing for O-03 to retire.
    ///
    /// It is worth being precise about a wording difference a reader will hit:
    /// `core/src/specification.rs` calls `PrecedenceSpecification` "a
    /// global-scope precedence obligation", meaning the obligation is quantified
    /// over every trigger in the Binding's Slice. Both are true and they are
    /// about different things — quantification is global, the Scope each
    /// individual obligation is discharged in is the trigger's own causal past.
    EventLocal,
    /// The Scope opens at one delimiter Event and closes at another
    /// (`docs/PRD.md` / ADR-0008's between-scope).
    ///
    /// Delimiters are **Pattern selector parameters**, not payload on this
    /// variant — the same discipline CAT-2 already used for `(after, until]`
    /// under [`ScopeKind::EventLocal`]. What Between buys is
    /// [`ScopeKind::always_closes`] `== false`, which forces a retiring
    /// [`RetirementPolicy`] under the catalogue biconditional. CAT-3
    /// `existence.before-scope-close` is the Pattern that spends this
    /// reservation (#210).
    ///
    /// Interval membership for the required Event is open-closed
    /// `(open, close]`. Concurrent open/required/close is observational
    /// uncertainty (ADR-0020).
    Between,
    /// The Scope is the whole Trace (ADR-0008's global scope).
    ///
    /// No Pattern in this catalogue declares it. #14a's first blocker is
    /// **removed**: a global Scope has no close, so [`ScopeKind::always_closes`]
    /// is `false` for it and the catalogue invariant demands a retirement
    /// policy — and since issue #110 that demand is satisfiable, because
    /// [`RetirementPolicy::Deadline`] and [`RetirementPolicy::Inactivity`] now
    /// ship.
    ///
    /// The second blocker was found on issue #141, is not about retirement at
    /// all, and **stands**. Retirement does not resolve it, and taking it here
    /// would be the drive-by semantic change ADR-0031 exists to stop; #210
    /// carries it as an explicit option.
    ///
    /// ADR-0025 licenses `Satisfy` only "when the Specification's Scope closes
    /// without a bad prefix". A Scope that never closes therefore never licenses
    /// a `Satisfy` — not late, not at a Watermark, not at all — so a globally
    /// scoped safety Pattern is one whose only settleable Verdict is `Violate`.
    /// That may well be the right Pattern to ship one day, but it is a product
    /// decision about what a permanently-open Specification means on the Verdict
    /// surface, and it has to be made rather than arrived at by declaring a
    /// Scope. Both CAT-1 and CAT-4 avoid the question by evaluating at an Event
    /// whose occurrence *is* the Scope's close.
    Global,
}

impl ScopeKind {
    /// Returns the Scope's identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EventLocal => "event-local",
            Self::Between => "between",
            Self::Global => "global",
        }
    }

    /// Returns whether this Scope is guaranteed to close.
    ///
    /// A Scope that cannot fail to close needs no retirement policy; a Scope
    /// that can needs one, which is O-03's obligation (issue #110).
    #[must_use]
    pub const fn always_closes(self) -> bool {
        match self {
            Self::EventLocal => true,
            Self::Between | Self::Global => false,
        }
    }
}

/// What a Pattern declares about Scopes that may never close (ADR-0021, O-03).
///
/// # The representation decision, answered rather than deferred
///
/// #14a reserved this module's deadline-representation decision for issue #110,
/// on the rule that a data-carrying variant is a design decision and waits for
/// the Pattern that needs it. #110 answers it: **a retirement policy is a
/// declaration, and the duration is a periphery parameter that is deliberately
/// not represented here.** [`RetirementPolicy::Deadline`] and
/// [`RetirementPolicy::Inactivity`] therefore ship as **unit** variants,
/// transcribed from `docs/PRD.md`'s O-03 wording — "a deadline or inactivity
/// policy" — and carrying no duration.
///
/// The reasons are two, and both are checkable here. First, the Verification
/// Core never measures the horizon: retirement arrives as an input carrying the
/// horizon the periphery already decided on, so a duration in the catalogue
/// would be a number no evaluator reads. Second, a duration would have to be a
/// declared parameter, and `every_declared_parameter_is_consumed_by_instantiation`
/// fails on a descriptor nothing consumes.
///
/// # What is not here
///
/// A watermark-bounded policy is **dropped rather than deferred** with the rest
/// of CAT-3. A Watermark is an operational finality boundary (ADR-0018) — it
/// means "the Core holds everything each required source claims to have
/// emitted", not "no further Event will arrive for this Scope" — so a
/// fully-advanced Watermark is a *lower bound on patience*, a periphery policy
/// input and never a retirement trigger. Deriving one from the other is exactly
/// the conflation issue #110 refuses. #210 records it as an option with that
/// reading attached.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RetirementPolicy {
    /// The Scope cannot fail to close, so no retirement mechanism applies.
    NotRequired,
    /// The Scope retires when the periphery's declared deadline passes.
    ///
    /// The deadline itself is a periphery parameter rather than a catalogue
    /// one: see this enum's header.
    Deadline,
    /// The Scope retires after the periphery's declared quiet window.
    ///
    /// The window itself is a periphery parameter, for
    /// [`RetirementPolicy::Deadline`]'s reason.
    Inactivity,
}

impl RetirementPolicy {
    /// Returns the policy's identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotRequired => "not-required",
            Self::Deadline => "deadline",
            Self::Inactivity => "inactivity",
        }
    }

    /// Returns whether this policy admits a retirement signal at all.
    ///
    /// The Verification Core's clause (d0) reads exactly this: a Specification
    /// declaring [`RetirementPolicy::NotRequired`] cannot be retired, and that
    /// refusal comes from the Pattern's own declaration rather than from a
    /// family check hardcoded into the evaluator.
    #[must_use]
    pub const fn is_retirable(self) -> bool {
        match self {
            Self::NotRequired => false,
            Self::Deadline | Self::Inactivity => true,
        }
    }
}

/// The tier a Pattern's instantiations are evaluated on (ADR-0005, ADR-0020).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum EvaluationTier {
    /// Evaluated on the hot path, from the Slice's own Events alone.
    Hot,
    /// Evaluated on the cold path, needing relational facts the hot path asks
    /// for and waits on.
    Cold,
}

impl EvaluationTier {
    /// Returns the tier's identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hot => "hot",
            Self::Cold => "cold",
        }
    }
}

/// The evaluation tier and expected per-Verdict cost a Pattern carries
/// (CONTEXT.md, *Cost Class*).
///
/// This is the **declared** half. `core/src/cost.rs` supplies the measured
/// actual — it counts cold-path fact requests per Verdict — and its module doc
/// assigns this declaration to #14/#15. The two are meant to be compared, as
/// ADR-0008 compares actual Filter width against declared budget.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct CostClass {
    /// The tier instantiations of this Pattern evaluate on.
    pub tier: EvaluationTier,
    /// Cold-path fact requests one Binding's Verdict is expected to cost.
    ///
    /// Zero for a hot Pattern, which is the honest declaration rather than an
    /// absent one: a hot Pattern claims it will never round-trip, and
    /// `core/src/cost.rs` can falsify that claim.
    pub expected_fact_requests_per_verdict: u32,
}

/// A Pattern's declared Filter-width budget plus whether it was ever observed.
///
/// ADR-0008 makes the budget "known from the Pattern before deployment and
/// reported in cost-class telemetry as actual width versus declared budget".
/// `measured` records which half of that comparison actually happened, because
/// a budget nobody has compiled against is a guess and should not read as a
/// finding. `over_budget` remains telemetry-only and never a gate: see
/// `crate::width`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct WidthBudget {
    /// The Pattern author's per-instance commitment.
    pub budget: FilterWidthBudget,
    /// `true` iff an instantiation of this Pattern has had its actual emitted
    /// width compared against this budget by a test in this repository.
    pub measured: bool,
    /// What was measured, or why nothing was.
    pub note: &'static str,
}

/// The kind of value a [`ParameterDescriptor`] accepts.
///
/// This is the schema #21 constrains generation with and #35 renders a form
/// from. It is deliberately small: `EventSelector` in the Core is kind plus
/// conjunctive attribute equality, with no negation and no ranges, and #14 does
/// not extend the selector language.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ParameterKind {
    /// An Event attribute key whose value names a Binding.
    AttributeKey,
    /// An Event kind plus the attribute valuations a matching Event must carry.
    Selector,
    /// A lookback measured in **sequence positions** (causal Event count along
    /// the ordered evidence), not wall-clock duration.
    ///
    /// Maps to cold-path `LookbackBound`. Dogwood `s/m/h/d` leaves are refused by
    /// the Tier-1 translator (#174), not silently lowered here.
    SequenceLookbackBound,
    /// A non-negative integer cap compared exclusively: count `< threshold`
    /// Holds under a complete lookback; count `≥ threshold` Fails.
    CountThreshold,
}

impl ParameterKind {
    /// Returns the kind's identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AttributeKey => "attribute-key",
            Self::Selector => "selector",
            Self::SequenceLookbackBound => "sequence-lookback-bound",
            Self::CountThreshold => "count-threshold",
        }
    }
}

/// One parameter an instantiation of a Pattern must bind.
///
/// Every declared parameter is consumed by [`instantiate`], and
/// `every_declared_parameter_is_consumed_by_instantiation` proves it — a
/// descriptor nobody reads would let #21 generate a value that silently does
/// nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ParameterDescriptor {
    /// The parameter's name, unique within its Pattern.
    pub name: &'static str,
    /// The kind of value it accepts.
    pub kind: ParameterKind,
    /// What binding this parameter changes about the Pattern's meaning.
    pub description: &'static str,
}

/// A source role whose evidence a Pattern's conclusion is not trustworthy
/// without (ADR-0022).
///
/// `minimum_tier` is what the **Specification demands**. Where the source is
/// actually placed is a deployment fact the evidence asserts, so a shortfall is
/// an [`InstanceWarning::TierShortfall`] at instantiation, never an error —
/// `core/src/specification.rs` states the same split on `RequiredSource`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct EvidenceRequirement {
    /// The role's name, unique within its Pattern, bound at instantiation.
    pub role: &'static str,
    /// The lowest assurance tier this conclusion can rest on.
    pub minimum_tier: SourceTier,
    /// Capabilities the bound source must hold for the conclusion to be sound.
    pub capabilities: &'static [SourceCapability],
    /// Why this role needs this tier and these capabilities.
    pub rationale: &'static str,
}

/// How well evidenced a Pattern is, recorded in the catalogue's own data.
///
/// A code comment saying "we made this one up" is not readable by #21 or #35;
/// this is.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum EvidenceGrade {
    /// A design partner supplied the policy, incident or trace behind it.
    ///
    /// **No Pattern in the first catalogue carries this grade.** Issue #1 closed
    /// on a maintainer decision to proceed on internal validation, and the one
    /// partner-adjacent thread in the corpus's provenance ledger is recorded as
    /// pending with no artifacts. The grade exists so that the absence is
    /// visible in the data and testable.
    PartnerEvidenced,
    /// An expert-authored scenario in the M0 corpus states the Pattern's shape.
    CorpusEvidenced,
    /// Declared with no scenario behind it, on an ADR's authority alone.
    Unevidenced,
}

impl EvidenceGrade {
    /// Returns the grade's identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PartnerEvidenced => "partner-evidenced",
            Self::CorpusEvidenced => "corpus-evidenced",
            Self::Unevidenced => "unevidenced",
        }
    }
}

/// A checkable citation from a Pattern to the artifact that grounds it (O-07).
///
/// `anchor` must occur verbatim in `document`, and a test opens the file and
/// checks. That is the whole point: it makes traceability mechanical instead of
/// rhetorical, and it makes a fabricated quotation a build failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct GroundingRef {
    /// Repo-root-relative path of the grounding document.
    pub document: &'static str,
    /// Text that must occur verbatim in `document`.
    pub anchor: &'static str,
    /// What this reference grounds, in this Pattern.
    pub supports: &'static str,
}

/// One Pattern, declared as data.
///
/// The four display fields — `name`, `summary`, `formula`, `scope_note` — are
/// exactly `dashboard/src/data/mock.ts`'s `Pattern` interface, and `shape`'s
/// string form is its `PatternShape` discriminant, so #35 renders this with no
/// redesign. `id` is the sixth thing #35 needs and the mock does not have: a
/// stable catalogue identity distinct from the temporal shape, because more than
/// one Pattern can share a shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct PatternDeclaration {
    /// Stable catalogue identity.
    pub id: PatternId,
    /// Display name.
    pub name: &'static str,
    /// One-sentence statement of the property.
    pub summary: &'static str,
    /// The temporal formula, in the notation ADR-0020 uses.
    ///
    /// Plain text rather than LaTeX. The dashboard mock renders LaTeX; turning
    /// this into the rendered form is #35's presentation concern, and inventing
    /// a second encoding here would be a second thing to keep in sync.
    pub formula: &'static str,
    /// How the Scope behaves, in prose, for a reviewer.
    pub scope_note: &'static str,
    /// The Dwyer shape.
    pub shape: TemporalShape,
    /// The Scope kind.
    pub scope: ScopeKind,
    /// What this Pattern declares about Scopes that may never close.
    pub retirement: RetirementPolicy,
    /// Parameters an instantiation must bind.
    pub parameters: &'static [ParameterDescriptor],
    /// Source roles an instantiation must bind, and what each must hold.
    pub evidence: &'static [EvidenceRequirement],
    /// Declared Filter-width budget, lifted from the instance to the template.
    pub width: WidthBudget,
    /// Declared evaluation tier and per-Verdict cost.
    pub cost: CostClass,
    /// How well evidenced this Pattern is.
    pub grade: EvidenceGrade,
    /// Checkable citations to the artifacts that ground it.
    pub grounding: &'static [GroundingRef],
}

impl PatternDeclaration {
    /// Returns the descriptor of one declared parameter, if it exists.
    #[must_use]
    pub fn parameter(&self, name: &str) -> Option<&'static ParameterDescriptor> {
        self.parameters.iter().find(|entry| entry.name == name)
    }

    /// Returns the requirement for one declared evidence role, if it exists.
    #[must_use]
    pub fn requirement(&self, role: &str) -> Option<&'static EvidenceRequirement> {
        self.evidence.iter().find(|entry| entry.role == role)
    }
}

/// The Pattern catalogue.
///
/// Five Patterns. ADR-0019's acceptance criterion asks for three to five
/// including **both** flagship families; issue #141 supplies the second family
/// and the P-B absence Shape; issue #173 adds CAT-5 `aggregation.count-within`
/// (sequence-lookback neighbour to Dogwood aggregation — not Dogwood-as-specified
/// wall-clock parity; see `docs/research/o07-dogwood-parity.md`); issue #210
/// adds CAT-3 `existence.before-scope-close` with [`ScopeKind::Between`] and
/// [`RetirementPolicy::Deadline`], making O-03's evaluator half reachable from
/// catalogue data. Saying "five" plainly is the point.
#[must_use]
pub const fn catalogue() -> &'static [PatternDeclaration] {
    CATALOGUE
}

/// Returns the declaration for one Pattern.
#[must_use]
pub const fn declaration(id: PatternId) -> &'static PatternDeclaration {
    match id {
        PatternId::PrecedenceApproval => &CATALOGUE[0],
        PatternId::ProvenanceTaint => &CATALOGUE[1],
        PatternId::AbsenceAfterUntil => &CATALOGUE[2],
        PatternId::AggregationCountWithin => &CATALOGUE[3],
        PatternId::ExistenceBeforeScopeClose => &CATALOGUE[4],
    }
}

/// The parameter naming the Binding attribute of CAT-1.
pub const PRECEDENCE_BINDING_ATTRIBUTE: &str = "binding_attribute";
/// The parameter naming CAT-1's obligation-creating selector.
pub const PRECEDENCE_TRIGGER: &str = "trigger";
/// The parameter naming CAT-1's obligation-discharging selector.
pub const PRECEDENCE_REQUIREMENT: &str = "requirement";
/// The evidence role that witnesses CAT-1's trigger.
pub const PRECEDENCE_TRIGGER_SOURCE: &str = "trigger_source";
/// The evidence role that witnesses CAT-1's requirement.
pub const PRECEDENCE_REQUIREMENT_SOURCE: &str = "requirement_source";

/// The parameter naming the Binding attribute of CAT-4.
pub const TAINT_BINDING_ATTRIBUTE: &str = "binding_attribute";
/// The parameter naming CAT-4's obligation-raising selector.
pub const TAINT_ACTION: &str = "action";
/// The parameter naming CAT-4's falsifying selector.
pub const TAINT_UNTRUSTED_CONTENT: &str = "untrusted_content";
/// The evidence role that witnesses CAT-4's action.
pub const TAINT_ACTION_SOURCE: &str = "action_source";
/// The evidence role that witnesses CAT-4's untrusted content.
pub const TAINT_CONTENT_SOURCE: &str = "content_source";

/// The parameter naming the Binding attribute of CAT-2.
pub const ABSENCE_BINDING_ATTRIBUTE: &str = "binding_attribute";
/// The parameter naming CAT-2's opening delimiter.
pub const ABSENCE_AFTER: &str = "after";
/// The parameter naming CAT-2's obligation-raising close.
pub const ABSENCE_UNTIL: &str = "until";
/// The parameter naming CAT-2's prohibited Event inside `(after, until]`.
pub const ABSENCE_VETO: &str = "veto";
/// The evidence role that witnesses CAT-2's opening delimiter.
pub const ABSENCE_AFTER_SOURCE: &str = "after_source";
/// The evidence role that witnesses CAT-2's obligation-raising close.
pub const ABSENCE_UNTIL_SOURCE: &str = "until_source";
/// The evidence role that witnesses CAT-2's veto.
pub const ABSENCE_VETO_SOURCE: &str = "veto_source";

/// CAT-3 parameter: attribute whose value names a Binding.
pub const EXISTENCE_BINDING_ATTRIBUTE: &str = "binding_attribute";
/// CAT-3 parameter: Events that open the Between Scope and raise the obligation.
pub const EXISTENCE_OPEN: &str = "open";
/// CAT-3 parameter: Events that must occur inside `(open, close]`.
pub const EXISTENCE_REQUIRED: &str = "required";
/// CAT-3 parameter: Events that close the Between Scope.
pub const EXISTENCE_CLOSE: &str = "close";
/// CAT-3 evidence role: source of the obligation-raising open Event.
pub const EXISTENCE_OPEN_SOURCE: &str = "open_source";
/// CAT-3 evidence role: source of the required Event (anchor for absence-convict).
pub const EXISTENCE_REQUIRED_SOURCE: &str = "required_source";
/// CAT-3 evidence role: source of the close Event.
pub const EXISTENCE_CLOSE_SOURCE: &str = "close_source";

/// The parameter naming the Binding attribute of CAT-5.
pub const AGGREGATION_BINDING_ATTRIBUTE: &str = "binding_attribute";
/// The parameter naming CAT-5's obligation-raising assessment selector.
pub const AGGREGATION_TRIGGER: &str = "trigger";
/// The parameter naming CAT-5's counted member selector.
pub const AGGREGATION_MEMBER: &str = "member";
/// The parameter naming CAT-5's sequence lookback bound.
pub const AGGREGATION_LOOKBACK: &str = "lookback";
/// The parameter naming CAT-5's count threshold.
pub const AGGREGATION_THRESHOLD: &str = "threshold";
/// The evidence role that witnesses CAT-5's trigger/assessment Event.
pub const AGGREGATION_TRIGGER_SOURCE: &str = "trigger_source";
/// The evidence role that witnesses CAT-5's counted member Events.
pub const AGGREGATION_MEMBER_SOURCE: &str = "member_source";

const CORPUS: &str = "docs/research/m0-evidence-corpus.md";
const ADR_0005: &str = "docs/adr/0005-two-tier-verification-core.md";
const ADR_0008: &str = "docs/adr/0008-patterns-are-the-frontend-and-the-generation-grammar.md";
const ADR_0016: &str = "docs/adr/0016-cross-system-action-sequence-verification.md";
const ADR_0020: &str = "docs/adr/0020-partial-order-monitoring-over-causal-frontiers.md";
const ADR_0022: &str = "docs/adr/0022-assurance-tiered-probe-placement.md";
const ADR_0025: &str = "docs/adr/0025-patterns-compile-to-scoped-cut-lattice-evaluators.md";
const PRD: &str = "docs/PRD.md";

const CATALOGUE: &[PatternDeclaration] = &[
    PatternDeclaration {
        id: PatternId::PrecedenceApproval,
        name: "Causal precedence with approval Binding",
        summary: "A named Event never occurs unless a qualifying Event for the same Binding lies in its causal past.",
        formula: "AG(trigger -> EP(requirement))",
        scope_note: "Event-local Scope evaluated at the trigger Event, over its causal past only.",
        shape: TemporalShape::Precedence,
        scope: ScopeKind::EventLocal,
        retirement: RetirementPolicy::NotRequired,
        parameters: &[
            ParameterDescriptor {
                name: PRECEDENCE_BINDING_ATTRIBUTE,
                kind: ParameterKind::AttributeKey,
                description: "The Event attribute whose value names a Binding. Each Binding gets its own Slice and its own Verdict, which is what makes the property uncheckable by a local guardrail.",
            },
            ParameterDescriptor {
                name: PRECEDENCE_TRIGGER,
                kind: ParameterKind::Selector,
                description: "Events that create the obligation. One obligation per matching Event per Binding.",
            },
            ParameterDescriptor {
                name: PRECEDENCE_REQUIREMENT,
                kind: ParameterKind::Selector,
                description: "Events that discharge the obligation. A matching Event must lie in the trigger's causal past in every admissible linearization.",
            },
        ],
        evidence: &[
            EvidenceRequirement {
                role: PRECEDENCE_REQUIREMENT_SOURCE,
                minimum_tier: SourceTier::Anchor,
                capabilities: &[
                    SourceCapability::CausalEdges,
                    SourceCapability::CompletenessSignals,
                ],
                rationale: "An authoritative Violate rests on the absence of a requirement Event, and an absence has no witness of its own. The precedence monitor's anchor downgrade demotes exactly that Violate to Inconclusive(prefix) when no anchor-tier candidate exists, so a contributing-tier requirement source can never settle one. CausalEdges because the obligation is reachability; CompletenessSignals because the violate row fires only once the Watermark says no further requirement Event is outstanding.",
            },
            EvidenceRequirement {
                role: PRECEDENCE_TRIGGER_SOURCE,
                minimum_tier: SourceTier::Contributing,
                capabilities: &[],
                rationale: "The trigger witnesses itself: it is present in the evidence that raises the obligation, so contributing tier suffices. Requiring anchor here would exclude the ordinary case where the acting service is the one inside the Monitored Application.",
            },
        ],
        width: WidthBudget {
            budget: FilterWidthBudget {
                event_kinds: 2,
                attribute_keys: 4,
            },
            measured: true,
            note: "Two Event kinds (trigger, requirement) and four attribute keys (the Binding attribute plus up to three selector attributes). Observed for the SagaShop P4, P6 and P8 instantiations: two kinds and two keys, actual width 4 against declared 6. Read that as the one width that has been observed, not as a validated budget: the pre-existing observation in core/tests/filter_compile.rs is against a 1+1 budget that file calls deliberately tight, chosen to force the over-budget path.",
        },
        cost: CostClass {
            tier: EvaluationTier::Hot,
            expected_fact_requests_per_verdict: 0,
        },
        grade: EvidenceGrade::CorpusEvidenced,
        grounding: &[
            GroundingRef {
                document: CORPUS,
                anchor: "| P-A | Bounded causal precedence with approval Binding",
                supports: "The corpus's own Pattern-candidate row for this family, which fixes its tier (hot), its width (narrow) and its anchor sources (permission/policy system plus action gateway). CAT-1 is P-A.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "## S1 — Autonomous remediation with ephemeral privilege escrow",
                supports: "A mutation must be causally preceded by a blast-radius approval for the same remediation. The approval binds to the action, which is the Binding attribute parameter.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "## S2 — Cross-ledger settlement in algorithmic HFT",
                supports: "A fill must be causally preceded by a margin reservation. The reservation is anchor tier (risk system), the router that fills is contributing — the tier split this Pattern declares.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "## S5 — Zero-trust mesh dynamic microsegmentation and policy eviction",
                supports: "A payload may be processed only if the policy node evaluated an Allow for that request, and the corpus names this Scope Event-local — the trigger's causal past.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "No scenario in this corpus is backed by a captured Trace.",
                supports: "The reliability class of every scenario above, in the corpus's own words. It covers all five; the ledger's longer parenthetical is attached to S1 alone and is not quoted here for the others.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "evidences four (P-A through P-D) — three of them precedence/approval-shaped.",
                supports: "Why the grade is CorpusEvidenced rather than Unevidenced. The corpus classes P-A as evidenced; only P-E, provenance/taint, is a gap.",
            },
            GroundingRef {
                document: "docs/adr/0008-patterns-are-the-frontend-and-the-generation-grammar.md",
                anchor: "precedence/approval is narrow",
                supports: "Why the width budget is small. ADR-0008 pairs this family against provenance/taint as the two opposite-width flagships.",
            },
            GroundingRef {
                document: "core/src/precedence.rs",
                anchor: "source is anchor-tier, silence cannot anchor; stay",
                supports: "The rule that makes requirement_source's anchor minimum load-bearing: watermark-complete silence of a required *anchor-tier* source settles Violate; contributing-only silence cannot.",
            },
            GroundingRef {
                document: "core/tests/e2e_oracle.rs",
                anchor: "P7 is an oracle check because its requirement",
                supports: "The English caveat that InstanceWarning::TierShortfall mechanizes. SagaShop P7 is this Pattern with a contributing-tier requirement source, and the test comment says in prose what the warning now says in data.",
            },
            GroundingRef {
                document: "core/tests/support/mod.rs",
                anchor: "pub fn ship_requires_prior_charge() -> PrecedenceSpecification {",
                supports: "The hand-written testbed fixture this Pattern must reproduce exactly. core/tests/catalogue_grounding.rs asserts structural equality between it and an instantiation of CAT-1, which is what turns the P4/P6/P8 mapping from a claim into an assertion.",
            },
        ],
    },
    PatternDeclaration {
        id: PatternId::ProvenanceTaint,
        name: "Causal provenance with untrusted-content taint",
        summary: "A named Event never occurs when an Event marking untrusted content for the same Binding lies in its causal past.",
        formula: "AG(action -> !EP(untrusted_content))",
        scope_note: "Event-local Scope evaluated at the action Event, over its causal past only. The Scope's close is the action Event itself, which is what licenses a Satisfy under ADR-0025 rather than a Watermark doing it.",
        shape: TemporalShape::Reachability,
        scope: ScopeKind::EventLocal,
        retirement: RetirementPolicy::NotRequired,
        parameters: &[
            ParameterDescriptor {
                name: TAINT_BINDING_ATTRIBUTE,
                kind: ParameterKind::AttributeKey,
                description: "The Event attribute whose value names a Binding. ADR-0016's differentiator is that the prohibited sequence is data-parametric: the question is not whether untrusted content was read anywhere, but whether it reached this recipient's and this workflow's action.",
            },
            ParameterDescriptor {
                name: TAINT_ACTION,
                kind: ParameterKind::Selector,
                description: "Events whose provenance is constrained. One obligation per matching Event per Binding, and the selector the census keys on: an evidence set with taint markers and no action establishes nothing.",
            },
            ParameterDescriptor {
                name: TAINT_UNTRUSTED_CONTENT,
                kind: ParameterKind::Selector,
                description: "Events marking the ingestion of untrusted content. A matching Event falsifies the property when it lies in an action's causal past. It may match freely elsewhere in the Trace and usually does, which is why it is not the selector the census keys on.",
            },
        ],
        evidence: &[
            EvidenceRequirement {
                role: TAINT_ACTION_SOURCE,
                minimum_tier: SourceTier::Anchor,
                capabilities: &[SourceCapability::CausalEdges],
                rationale: "Deliberately stricter than CAT-1's trigger_source, and the difference is the polarity. CAT-1's Satisfy rests on a positive witness, so a contributing-tier trigger costs it nothing; this Pattern's Satisfy is a claim about the action's own ancestry — that nothing untrusted reached it. A source inside the Monitored Application declaring its own inputs is the suppression failure ADR-0022 names, and a Satisfy resting on it is an agent certifying that it read nothing it should not have. CausalEdges because the property is reachability into this Event, and an action reporting no ancestry at all would otherwise read as clean.",
            },
            EvidenceRequirement {
                role: TAINT_CONTENT_SOURCE,
                minimum_tier: SourceTier::Anchor,
                capabilities: &[
                    SourceCapability::CausalEdges,
                    SourceCapability::CompletenessSignals,
                ],
                rationale: "An authoritative Violate rests on this Event's presence in an action's causal past, and ADR-0022 is explicit that a contributing-tier source cannot alone anchor a Verdict for a high-assurance property. What the tier does not do here is discard the observation: the decision table in core/src/taint.rs tests presence first and tier second, so a contributing-tier-only witness downgrades a Violate to Inconclusive and can never reach Satisfy. CompletenessSignals because the Satisfy row is an absence claim about this source, and only a Watermark closes one.",
            },
        ],
        width: WidthBudget {
            budget: FilterWidthBudget {
                event_kinds: 2,
                attribute_keys: 8,
            },
            measured: false,
            note: "Two Event kinds (action, untrusted-content marker) and eight attribute keys: the Binding attribute, up to three selector attributes on each side, and room for the content-identity attribute a marker carries. Wide against CAT-1's six, which is ADR-0008's pairing of the two flagship families at opposite widths — and deliberately not wider, because this Pattern's taint predicate is an attribute on a marker Event rather than raw content. Nothing has been measured: no instantiation of CAT-4 has been compiled against a deployment, so this is a budget in ADR-0008's sense, known from the Pattern before deployment, and the first measurement should be expected to move it.",
        },
        cost: CostClass {
            tier: EvaluationTier::Hot,
            expected_fact_requests_per_verdict: 0,
        },
        grade: EvidenceGrade::Unevidenced,
        grounding: &[
            GroundingRef {
                document: ADR_0016,
                anchor: "influenced by untrusted content. Sentinel evaluates deterministic Specifications",
                supports: "The property this Pattern states, in the words of the ADR that defines Sentinel's differentiated claim. ADR-0016 gives two examples of a prohibited data-parametric sequence; CAT-1 is the first and this is the second.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "| P-E | Provenance/taint (content or capability lineage across systems)",
                supports: "The corpus's own Pattern-candidate row for this family. It records the family as the corpus's one gap and fixes its width as wide, which is why the grade below is Unevidenced and the budget above is larger than CAT-1's.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "#14 must either author P-E directly against",
                supports: "The sanction for authoring a Pattern with no scenario behind it. Declaring CAT-4 Unevidenced is the corpus's own first option taken deliberately, not a way around its second.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "exercises untrusted-content influence propagating across systems",
                supports: "What the corpus says is missing, which is what this Pattern is graded against. No scenario exercises it, so no scenario is cited here.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "S1's lease lineage as taint evidence; it is causal ancestry, which is past-time.",
                supports: "The constraint this entry obeys. S1 grounds CAT-1 and appears nowhere in this list; nothing here presents causal ancestry from another scenario as evidence for taint.",
            },
            GroundingRef {
                document: ADR_0020,
                anchor: "provenance / taint reachability is cold",
                supports: "Why the tier declared above needs an argument rather than a citation. What ADR-0020 costs as cold is propagating taint — a path with no sanitizer on it, which is a since-like operator over the causal DAG. ADR-0016's example needs only !EP(untrusted) over the causal ancestry CAT-1 already walks, so CAT-4 is the non-propagating member of the family and stays hot inside the PaBTL fragment.",
            },
            GroundingRef {
                document: ADR_0025,
                anchor: "no user-facing Pattern exposes one until a partner property",
                supports: "Why the propagating member is not what shipped. O-20b defers EY-class operators until a partner property requires them and none exists, so CAT-4 declares no operator outside the hot fragment and asks the cold path for nothing.",
            },
            GroundingRef {
                document: ADR_0025,
                anchor: "Specification remains open; `Satisfy` settles",
                supports: "The licence the Scope above is chosen to satisfy. Evaluating at the action Event makes the Scope's close the same Event that raised the obligation, so a Satisfy is licensed exactly where ADR-0025 licenses it and never by a Watermark alone.",
            },
            GroundingRef {
                document: ADR_0022,
                anchor: "but cannot, alone, anchor a Verdict for a",
                supports: "The tier condition both evidence roles above rest on, and the one the decision table must apply without letting it reach a Satisfy.",
            },
            GroundingRef {
                document: "core/src/taint.rs",
                anchor: "presence is tested above tier, and the tier test cannot reach Satisfy",
                supports: "The decision table itself. It is the artifact that makes the two rationales above checkable rather than asserted, and it carries the argument for why folding the tier condition into the violation row is harmless for CAT-1 and a silent pass here.",
            },
        ],
    },
    PatternDeclaration {
        id: PatternId::AbsenceAfterUntil,
        name: "Absence between after and until",
        summary: "A named veto Event never occurs inside the open interval (after, until] for the same Binding.",
        formula: "AG(until -> !EP_(after,until](veto))",
        scope_note: "Event-local Scope evaluated at the until Event: the Scope opens at after and closes at until. Delimiters are selector parameters, not a ScopeKind::Between payload; RetirementPolicy::NotRequired holds because until is the close.",
        shape: TemporalShape::Absence,
        scope: ScopeKind::EventLocal,
        retirement: RetirementPolicy::NotRequired,
        parameters: &[
            ParameterDescriptor {
                name: ABSENCE_BINDING_ATTRIBUTE,
                kind: ParameterKind::AttributeKey,
                description: "The Event attribute whose value names a Binding. Each Binding gets its own Slice and its own Verdict: the question is whether a veto reached this Binding's open interval, not whether a veto occurred anywhere.",
            },
            ParameterDescriptor {
                name: ABSENCE_AFTER,
                kind: ParameterKind::Selector,
                description: "Events that open the interval. Present in evidence as a delimiter; contributing-tier witness suffices, as with CAT-1's trigger. Reported as the interval's opening count, not as the census key.",
            },
            ParameterDescriptor {
                name: ABSENCE_UNTIL,
                kind: ParameterKind::Selector,
                description: "Events that close the interval and raise the absence obligation. One obligation per matching Event per Binding, and the selector the census keys on: an evidence set full of vetoes and no until establishes nothing.",
            },
            ParameterDescriptor {
                name: ABSENCE_VETO,
                kind: ParameterKind::Selector,
                description: "Events prohibited inside (after, until]. A matching Event falsifies the property when it lies in that open interval. It may match freely outside it and usually does, which is why it is not the selector the census keys on.",
            },
        ],
        evidence: &[
            EvidenceRequirement {
                role: ABSENCE_UNTIL_SOURCE,
                minimum_tier: SourceTier::Anchor,
                capabilities: &[SourceCapability::CausalEdges],
                rationale: "Like CAT-4's action_source: the obligation-raising close is where Satisfy is claimed about an interval that must contain no veto. CausalEdges because whether a veto lies inside (after, until] is a reachability question into this Event's past relative to the opening delimiter. A contributing-tier until source declaring its own interval clean is the suppression failure ADR-0022 names.",
            },
            EvidenceRequirement {
                role: ABSENCE_VETO_SOURCE,
                minimum_tier: SourceTier::Anchor,
                capabilities: &[
                    SourceCapability::CausalEdges,
                    SourceCapability::CompletenessSignals,
                ],
                rationale: "An authoritative Violate rests on this Event's presence inside the open interval, and ADR-0022 is explicit that a contributing-tier source cannot alone anchor a Verdict for a high-assurance property. CompletenessSignals because a Satisfy is an absence claim about this source inside the interval, and only a Watermark closes one. CausalEdges because the interval is causal, not wall-clock.",
            },
            EvidenceRequirement {
                role: ABSENCE_AFTER_SOURCE,
                minimum_tier: SourceTier::Contributing,
                capabilities: &[SourceCapability::CompletenessSignals],
                rationale: "The opening delimiter witnesses itself: it is present in the evidence that bounds the interval, so contributing tier suffices — the same split CAT-1 declares for trigger_source. CompletenessSignals are required because settled never-opened is NoObligation (an absence settlement about `after`), and without a completeness role on that source the pass would be silent. Requiring anchor here would exclude the ordinary case where the service that opens the interval sits inside the Monitored Application.",
            },
        ],
        width: WidthBudget {
            budget: FilterWidthBudget {
                event_kinds: 3,
                attribute_keys: 9,
            },
            measured: false,
            note: "Three Event kinds (after, until, veto) and nine attribute keys: the Binding attribute plus up to roughly three selector attributes on each side. Narrow-to-medium against CAT-1's six and short of CAT-4's ten, which matches the corpus's P-B width as narrow. Nothing has been measured: no instantiation of CAT-2 has been compiled against a deployment, so this is a budget in ADR-0008's sense.",
        },
        cost: CostClass {
            tier: EvaluationTier::Hot,
            expected_fact_requests_per_verdict: 0,
        },
        grade: EvidenceGrade::CorpusEvidenced,
        grounding: &[
            GroundingRef {
                document: CORPUS,
                anchor: "| P-B | Absence between/after (revocation or rejection veto: \"no veto Event between approval and action\")",
                supports: "The corpus's own Pattern-candidate row for this family, which fixes its tier (hot), its width (narrow) and its anchor sources (veto-emitting system). CAT-2 is P-B.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "## S1 — Autonomous remediation with ephemeral privilege escrow",
                supports: "No Revoke between approval and mutation — the corpus's absence-between shape with approval as after, mutation as until, and Revoke as veto.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "## S4 — Distributed saga with asynchronous compensations (supply chain)",
                supports: "No CapturePayment after CarrierReject — the corpus's absence-after shape, with rejection as the opening delimiter and capture as the Event that must not occur once the interval is open.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "## S5 — Zero-trust mesh dynamic microsegmentation and policy eviction",
                supports: "No ThreatScoreElevated for the principal inside the request window — absence in window, Event-local at the processing close.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "## S3 — AV fleet teleoperation handover",
                supports: "No DegradedVideoFrameLoss since HandoverAck — absence after an opening acknowledgement, closed at actuation.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "No scenario in this corpus is backed by a captured Trace.",
                supports: "The reliability class of every scenario above, in the corpus's own words.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "evidences four (P-A through P-D) — three of them precedence/approval-shaped.",
                supports: "Why the grade is CorpusEvidenced rather than Unevidenced. The corpus classes P-B as evidenced alongside P-A.",
            },
            GroundingRef {
                document: ADR_0025,
                anchor: "Specification remains open; `Satisfy` settles",
                supports: "The licence the Scope above is chosen to satisfy. Evaluating at the until Event makes the Scope's close the same Event that raised the obligation, so a Satisfy is licensed exactly where ADR-0025 licenses it.",
            },
            GroundingRef {
                document: ADR_0022,
                anchor: "but cannot, alone, anchor a Verdict for a",
                supports: "The tier condition until_source and veto_source rest on.",
            },
        ],
    },
    PatternDeclaration {
        id: PatternId::AggregationCountWithin,
        name: "Count within sequence lookback",
        summary: "At a trigger Event, the count of matching member Events inside a sequence lookback for the same Binding must stay strictly below a threshold.",
        formula: "AG(trigger -> count_within(lookback, member) < threshold)",
        scope_note: "Event-local Scope evaluated at the trigger/assessment Event. Dogwood's within-W is a sequence lookback parameter on the Pattern (cold EvidenceWindow), not Scope, not Deadline, and not wall-clock duration.",
        shape: TemporalShape::Aggregation,
        scope: ScopeKind::EventLocal,
        retirement: RetirementPolicy::NotRequired,
        parameters: &[
            ParameterDescriptor {
                name: AGGREGATION_BINDING_ATTRIBUTE,
                kind: ParameterKind::AttributeKey,
                description: "The Event attribute whose value names a Binding. Each Binding gets its own Slice and its own Verdict: the rate cap is per Binding, not a global count across principals.",
            },
            ParameterDescriptor {
                name: AGGREGATION_TRIGGER,
                kind: ParameterKind::Selector,
                description: "Events that raise the aggregation obligation (the assessment / privileged action). One obligation per matching Event per Binding, and the selector the census keys on: an evidence set full of members and no trigger establishes nothing.",
            },
            ParameterDescriptor {
                name: AGGREGATION_MEMBER,
                kind: ParameterKind::Selector,
                description: "Events counted inside the sequence lookback. Their count is compared to threshold once the cold path holds a complete lookback. Members may match freely and must never be the census key — a clean under-threshold deployment would otherwise report as never exercised.",
            },
            ParameterDescriptor {
                name: AGGREGATION_LOOKBACK,
                kind: ParameterKind::SequenceLookbackBound,
                description: "How many sequence positions of causal evidence to include when counting members. Sequence positions, not wall-clock seconds/minutes/hours/days; Dogwood duration leaves are refused by Tier-1 (#174), not silently lowered.",
            },
            ParameterDescriptor {
                name: AGGREGATION_THRESHOLD,
                kind: ParameterKind::CountThreshold,
                description: "Exclusive upper bound on the member count inside the lookback: complete lookback with count < threshold may settle Satisfy after cold Holds; count ≥ threshold Fails the guard (Authoritative Inconclusive in PR-1 until #20).",
            },
        ],
        evidence: &[
            EvidenceRequirement {
                role: AGGREGATION_TRIGGER_SOURCE,
                minimum_tier: SourceTier::Anchor,
                capabilities: &[SourceCapability::CausalEdges],
                rationale: "Like CAT-4's action_source: the obligation-raising assessment is where Satisfy is claimed about a lookback that must stay under threshold. CausalEdges because the lookback is ordered over causal evidence into this Event. A contributing-tier trigger declaring its own rate-cap clean is the suppression failure ADR-0022 names.",
            },
            EvidenceRequirement {
                role: AGGREGATION_MEMBER_SOURCE,
                minimum_tier: SourceTier::Anchor,
                capabilities: &[
                    SourceCapability::CausalEdges,
                    SourceCapability::CompletenessSignals,
                ],
                rationale: "The aggregate is counted from this source's Events. CompletenessSignals because an incomplete or reaped lookback must never Holds (AMEND 4 / missing evidence is never a pass). CausalEdges because membership in the lookback is causal order, not wall-clock. ADR-0022: a contributing-tier-only member stream cannot alone anchor a high-assurance Verdict.",
            },
        ],
        width: WidthBudget {
            budget: FilterWidthBudget {
                event_kinds: 2,
                attribute_keys: 8,
            },
            measured: false,
            note: "Two Event kinds (trigger, member) and eight attribute keys: the Binding attribute plus up to roughly three selector attributes on each side. Medium width; nothing has been measured — no instantiation of CAT-5 has been compiled against a deployment, so this is a budget in ADR-0008's sense.",
        },
        cost: CostClass {
            tier: EvaluationTier::Cold,
            expected_fact_requests_per_verdict: 1,
        },
        grade: EvidenceGrade::Unevidenced,
        grounding: &[
            GroundingRef {
                document: ADR_0005,
                anchor: "graph, and for aggregation across Bindings. Both classes are load-bearing for the product.",
                supports: "ADR-0005 already names Watermark-consistent aggregates as cold-path work; CAT-5 is the Catalogue Pattern family that declares evidence, Filter-width budget, and Cost Class for that job.",
            },
            GroundingRef {
                document: PRD,
                anchor: "| **O-07** | The catalogue's actual contents.",
                supports: "O-07 is the catalogue-parity requirement this Pattern answers: expressiveness of the catalogue's actual contents.",
            },
            GroundingRef {
                document: PRD,
                anchor: "catalogue parity for the Dogwood fragment — aggregation",
                supports: "The Dogwood-parity gap (#173) naming aggregation Patterns as the mitigation for O-07 expressiveness.",
            },
            GroundingRef {
                document: PRD,
                anchor: "proposed as sequenced tickets #173 / #174",
                supports: "Issue #173 is the catalogue act; #174 (Tier-1 translator) stays blocked on this family.",
            },
            GroundingRef {
                document: PRD,
                anchor: "against #84's",
                supports: "Competitive research #84 is the origin of the Dogwood parity table this Pattern closes the hard gap of.",
            },
            GroundingRef {
                document: ADR_0008,
                anchor: "Anything outside it needs a new pattern, which is a",
                supports: "Adding a Pattern is a reviewed ADR-0008 act (golden snapshot diff). CAT-5 is that act for the aggregation family.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "No scenario in this corpus is backed by a captured Trace.",
                supports: "Why the grade is Unevidenced: no corpus scenario exercises a rate-cap / count-within property, and none is cited as if it did. No partner claims.",
            },
        ],
    },

    PatternDeclaration {
        id: PatternId::ExistenceBeforeScopeClose,
        name: "Existence before Scope close",
        summary: "A named Event occurs at least once inside a Between Scope before that Scope closes, for the same Binding.",
        formula: "EF_Scope(required)",
        scope_note: "Between Scope: opens at open, closes at close. Delimiters and the required Event are selector parameters, not ScopeKind payload. Declares RetirementPolicy::Deadline (S4 / P-D unreliable termination); Inactivity is grammar-only this PR and is not this Pattern's declaration (S3 hung-session inactivity stays recorded elsewhere). Interval membership is (open, close]; census keys on open only. Detect-only: ADR-0023 gates 4/5 unclaimed.",
        shape: TemporalShape::Existence,
        scope: ScopeKind::Between,
        retirement: RetirementPolicy::Deadline,
        parameters: &[
            ParameterDescriptor {
                name: EXISTENCE_BINDING_ATTRIBUTE,
                kind: ParameterKind::AttributeKey,
                description: "The Event attribute whose value names a Binding. Each Binding gets its own Slice and its own Verdict.",
            },
            ParameterDescriptor {
                name: EXISTENCE_OPEN,
                kind: ParameterKind::Selector,
                description: "Events that open the Between Scope and raise the existence obligation. The census keys on this selector only — never on required alone, which would vacuous-pass a Trace full of compensations with no saga open.",
            },
            ParameterDescriptor {
                name: EXISTENCE_REQUIRED,
                kind: ParameterKind::Selector,
                description: "Events that must occur at least once inside (open, close]. Their presence discharges the obligation; their absence at a settled close is the Violate.",
            },
            ParameterDescriptor {
                name: EXISTENCE_CLOSE,
                kind: ParameterKind::Selector,
                description: "Events that close the Between Scope. ADR-0025 licenses Satisfy only when the Scope closes without a bad prefix; retirement while still open pins structural Inconclusive rather than inventing a Satisfy.",
            },
        ],
        evidence: &[
            EvidenceRequirement {
                role: EXISTENCE_OPEN_SOURCE,
                minimum_tier: SourceTier::Contributing,
                capabilities: &[],
                rationale: "The open Event witnesses itself: it is present in the evidence that raises the obligation, so contributing tier suffices — the same split CAT-1 uses for its trigger.",
            },
            EvidenceRequirement {
                role: EXISTENCE_REQUIRED_SOURCE,
                minimum_tier: SourceTier::Anchor,
                capabilities: &[
                    SourceCapability::CausalEdges,
                    SourceCapability::CompletenessSignals,
                ],
                rationale: "An authoritative Violate rests on the absence of a required Event inside (open, close], and an absence has no witness of its own. Anchor tier plus CompletenessSignals: contributing-only silence cannot convict; CausalEdges because membership is reachability inside the Between interval.",
            },
            EvidenceRequirement {
                role: EXISTENCE_CLOSE_SOURCE,
                minimum_tier: SourceTier::Contributing,
                capabilities: &[SourceCapability::CausalEdges],
                rationale: "The close Event witnesses itself when present. CausalEdges because open-before-close ordering is the Scope's close condition. Contributing tier suffices for presence; Satisfy still needs Coverage closed and no bad prefix (ADR-0025).",
            },
        ],
        width: WidthBudget {
            budget: FilterWidthBudget {
                event_kinds: 3,
                attribute_keys: 4,
            },
            measured: false,
            note: "Three Event kinds (open, required, close) and four attribute keys (the Binding attribute plus room for selector attributes). measured=false until a SagaShop / vendored fixture exercises the catalogued instance end-to-end (#210 AC3 honesty residual B).",
        },
        cost: CostClass {
            tier: EvaluationTier::Hot,
            expected_fact_requests_per_verdict: 0,
        },
        grade: EvidenceGrade::CorpusEvidenced,
        grounding: &[
            GroundingRef {
                document: CORPUS,
                anchor: "| P-D | Compensation ordering (\"compensation precedes termination on abort\")",
                supports: "The corpus's Pattern-candidate row for this family: hot, narrow, and the one that requires deadline/inactivity because termination is the unreliable close.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "## S4 — Distributed saga with asynchronous compensations (supply chain)",
                supports: "S4 is the O-03 / P-D scenario: compensation must precede unreliable termination inside an open-ended Scope.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "evidences four (P-A through P-D) — three of them precedence/approval-shaped.",
                supports: "Why the grade is CorpusEvidenced: the corpus classes P-D as evidenced.",
            },
            GroundingRef {
                document: CORPUS,
                anchor: "S4 should be the O-03 test scenario.",
                supports: "Why Deadline is the declared retirement policy: S4 makes lease/saga deadline load-bearing.",
            },
            GroundingRef {
                document: ADR_0025,
                anchor: "Specification remains open; `Satisfy` settles",
                supports: "Satisfy is licensed only when the Scope closes without a bad prefix — retirement while open is structural Inconclusive, never a demotion of a Satisfy that was never licensed.",
            },
            GroundingRef {
                document: PRD,
                anchor: "Slice retirement for scopes without a reliable close Event",
                supports: "O-03's product requirement this Pattern makes reachable from catalogue data.",
            },
        ],
    },

];
