//! Same-family Specification subsumption: whether a Candidate can add any
//! Violate a deployed Specification does not already reach (issue #177,
//! ADR-0037).
//!
//! # The question, and its polarity
//!
//! A Candidate `C` is **subsumed by** a deployed Specification `S` when every
//! `(Trace, Binding)` on which `C`'s monitor settles a `Violate` outcome is one
//! on which `S`'s monitor settles a `Violate` outcome too. Promoting a subsumed
//! Candidate buys no Violate the deployment does not already reach, and still
//! costs per-anchor licence margin (ADR-0028).
//!
//! This is **Violate-set inclusion**. It is not Cedar's `check_implies`, which
//! is Allow-set inclusion over a point-in-time request: the direction of the
//! subset and the meaning of an element both differ. An element here is a
//! Trace — a causal DAG of Events — plus the Binding it is evaluated under. No
//! Allow, no Deny, no request.
//!
//! # Which `Violate`: the monitor's own outcome, and the limits that follow
//!
//! "Settles a `Violate` outcome" means the monitor's hot assessment —
//! `core::precedence::BindingOutcome::Violate` for CAT-1,
//! `core::taint::TaintOutcome::Violate` for CAT-4 — **before Verdict
//! assembly**, of each side's **IR-lowered** Specification — the one
//! `TryFrom<&PrecedenceIr>` / `TryFrom<&TaintIr>` builds in `core`. That is
//! the only reading derivable from IR. Four things stand between that outcome
//! and the Verdict a Specification finally emits, and this crate can see none
//! of them. The first three are Core-side inputs set on the Specification
//! after lowering, which never reach the IR (`core/src/specification.rs`:
//! rollout state does not belong in the IR):
//!
//! - **`activation_barrier` caps authority only.** `core/src/settlement.rs`
//!   computes `Provisional` from a pending barrier and says in terms that it
//!   does not suppress the Verdict. It never changes an outcome.
//! - **`cold_guards` can change the outcome.** When one of a Binding's
//!   cold-guard queries settles `Refused`, `core/src/verdict.rs`'s
//!   `epistemic_from_refused` rewrites the hot outcome — `Violate` included —
//!   to `Inconclusive(EpistemicUncertainty)`, on both the CAT-1 and the CAT-4
//!   path. Issue #20 will add a cold-`Fails` flip for CAT-1 beside it.
//! - **A retirement override can change the outcome.**
//!   `PrecedenceSpecification::with_retirement_policy` (`core/src/specification.rs`)
//!   replaces the retirement the Pattern declared, and a retired Binding's
//!   outcome is fixed when it retires. Only tests call it today, and CAT-4's
//!   Specification has no such builder; a deployed CAT-1 Specification built
//!   with it is not the IR-lowered one this analysis reasons about.
//!
//! The fourth is a deployment fact:
//!
//! - **Verdict assembly validates what a `Violate` cites.** A decisive witness
//!   Event must come from a source the Specification requires and observed
//!   (`validate_decisive_event` in `model/src/verdict.rs`), and the Core
//!   surfaces a refusal as an evaluation error, not as a Verdict. Nothing ties
//!   an Event kind to a source, so where `S`'s wider selector matches an Event
//!   from a source outside the required set both sides share, `S`'s monitor
//!   reaches the `Violate` and its evaluation fails instead of emitting it,
//!   while `C` — not matching that Event — emits its own.
//!
//! So [`Subsumption::Subsumed`] licenses exactly "`C`'s monitor reaches no
//! `Violate` outcome `S`'s monitor does not". It says nothing about authority,
//! and it does **not** say that `S` *emits* a `Violate` Verdict wherever `C`
//! would: a deployed Specification that declares cold guards can still leave
//! that Binding `Inconclusive` when a guard is refused, one built with a
//! retirement override is outside the argument, and one whose selectors match
//! Events from unrequired sources can fail to assemble the Verdict at all. The
//! caller that turns a proof into a Promotion blocker (a later call site on
//! #177) owns that choice: it should not key on [`Subsumption::is_proven`]
//! alone, and should not refuse a Candidate as subsumed by a Specification
//! that declares cold guards or overrides retirement unless it accepts this
//! knowingly. The cold-guard and witness-source limits are pinned by
//! execution, not by this paragraph —
//! `a_refused_cold_guard_leaves_the_deployed_specification_inconclusive_where_the_candidate_violates`
//! and
//! `a_wider_deployed_selector_matching_an_event_from_an_unrequired_source_fails_verdict_assembly`,
//! both in `core/tests/subsumption_cross_check.rs`.
//!
//! # Well-posedness: three preconditions, in a fixed order
//!
//! Checked before any family argument, in this order, and the first to fail
//! is reported as [`NotComparable`] — no evidence will ever make such a pair
//! comparable by this analysis:
//!
//! 1. **Same [`PatternId`].** Cross-family pairs are not comparable, which is a
//!    different statement from "not subsumed": CAT-1's `Violate` rests on an
//!    absence and CAT-4's on a presence (`crate::ir`'s header), and opposite
//!    polarity is not inclusion in either direction. The key is `PatternId`
//!    equality rather than family equality on purpose: a second `precedence.*`
//!    Pattern would carry its own parameters and decision table, and comparing
//!    by family would silently inherit CAT-1's polarity.
//! 2. **Same Binding attribute.** Slice membership is "the Event names a
//!    Binding under the Specification's own Binding attribute", independent of
//!    every selector (`core/src/evaluator.rs`). Equal attributes give `C` and
//!    `S` identical Slices and identical Binding sets, which is what the whole
//!    argument below stands on. Different ones mean the two Violate-sets hold
//!    different kinds of element.
//! 3. **Same producer contract.** The `SchemaVersion` fixes what an Event kind
//!    and an attribute key denote (`catalogue/instance.rs`); two selectors
//!    written against different contracts are written in different vocabularies.
//!
//! Specification identity is **not** a precondition. `specification_id` and
//! `specification_version` travel into every Assurance Envelope but change no
//! outcome, so an instance identical to a deployed Specification under another
//! identity is subsumed — the plainest subsumed pair.
//!
//! # The family arguments
//!
//! Selector inclusion is decided by
//! [`SelectorIr::narrowing_divergence`](crate::ir::SelectorIr::narrowing_divergence),
//! which is exact for this selector language and forces kind equality on every
//! role it admits. That kind equality is what makes the tier machinery drop out:
//! CAT-1's anchor candidates and CAT-4's anchor witness are both "Events of the
//! selector's **kind** from an anchor-tier source in this Slice", so once kinds
//! are equal they are the same set for `C` and `S`.
//!
//! The direction of inclusion is **not** uniform within a Pattern, and getting
//! it backwards is the defect this module is arranged around.
//!
//! **CAT-1 `precedence.approval`** — `Violate` is a trigger `t` with no
//! requirement Event in its causal past (neither `Before` nor `Concurrent`), the
//! Watermark covering the epoch, and either an anchor candidate present or the
//! watermark-anchored silence of a required anchor-tier source (ADR-0033).
//!
//! | role | direction | why |
//! |---|---|---|
//! | `trigger` | Candidate narrower | every obligation `C` raises must be one `S` raises |
//! | `requirement` | **deployed narrower** | if nothing in `C`'s wider discharging set lies in `t`'s past, nothing in `S`'s narrower set does either |
//! | required sources | **identity sets equal** | not monotone: more required sources close the evidence gate sooner, fewer can falsify the silent-anchor condition |
//!
//! The contravariance on `requirement` is the difficulty: narrowing the
//! discharging selector makes a Specification violate **more**, so "the
//! Candidate is more specific everywhere" is the wrong argument for CAT-1 and
//! the most likely implementation bug.
//!
//! **CAT-4 `provenance.taint`** — `Violate` is an action `t` with an
//! untrusted-content Event `u ->* t`, an open evidence gate, and an anchor-tier
//! Event of the untrusted-content kind in the Slice.
//!
//! | role | direction | why |
//! |---|---|---|
//! | `action` | Candidate narrower | as CAT-1's trigger |
//! | `untrusted_content` | **Candidate narrower** | `Violate` rests on the *presence* of `u`, so every `u` that falsifies `C` must falsify `S` |
//! | required sources | **identity sets equal** | the monitor alone would allow deployed ⊆ Candidate; Verdict assembly does not (below) |
//!
//! Both selectors narrower — the opposite of CAT-1's second role, which is the
//! inversion `crate::ir` refuses to express as a boolean field and the reason
//! this module matches on [`PatternId`] rather than iterating "the selectors".
//!
//! CAT-4's evidence condition is equality, and that is a finding rather than
//! caution. The monitor's only dependence on the required set is the evidence
//! gate, and the gate is monotone in it, so at the monitor level a deployed
//! Specification requiring a *subset* of the Candidate's sources would be
//! sound — the converged plan on #177 took that `⊆` on the condition that the
//! cross-check executed it. It did, and refuted it one layer up: the Violate
//! rests on an untrusted-content Event from the content source, which is
//! exactly the source a smaller deployed set drops, and Verdict assembly
//! refuses a decisive witness from an unrequired source. The deployed
//! Specification's monitor reaches the Violate and its evaluation errors
//! (`a_deployed_specification_requiring_fewer_sources_cannot_emit_the_violate_its_monitor_reaches`).
//! Equality keeps witness validation identical on both sides for every Event
//! both sides cite.
//!
//! Required sources compare by **identity only**. The minimum tier the IR
//! carries per source is unread on every decision path (`minimum_tier_of` has
//! no reader in `core/src`) — both tables read the
//! deployment's placement through the evidence ledger instead — so a tier
//! difference cannot change either Violate-set and is not reported.
//!
//! Retirement is compared for equality, and a difference is
//! [`Undecided::RetirementDiffers`]. No catalogued pair differs today — the
//! catalogue lowers one retirement per Pattern, and both CAT-1 and CAT-4
//! declare that no retirement applies — so the check is defensive: it keeps a
//! Pattern whose retirement becomes a parameter from being proven by an
//! argument that never read it.
//!
//! **Every IR field has a stated disposition.** Both family arguments, and
//! [`SelectorIr::narrowing_divergence`](crate::ir::SelectorIr::narrowing_divergence),
//! destructure their IR with no `..` and say per field whether it is compared,
//! checked in the prologue, or identity only. A field added to `PrecedenceIr`,
//! `TaintIr` or `SelectorIr` is therefore a compile error where the inclusion
//! argument lives, not a field a proof silently ignores — the same forcing
//! function the [`PatternId`] dispatch below applies to a new Pattern.
//!
//! **CAT-2, CAT-3 and CAT-5 are not derived.** Each carries a dimension whose
//! monotonicity lives in its own Core decision table — CAT-2's `(after, until]`
//! interval, CAT-3's `(open, close]` plus a deadline on a Scope that can fail
//! to close, CAT-5's `threshold` and sequence lookback. Deriving them here from
//! the IR's documentation would be a second copy of those tables in a second
//! crate, which ADR-0027 and issue #112 forbid for the same reason `crate::ir`
//! keeps the one matcher. They report [`Undecided::PolarityNotDerived`], one
//! [`PatternId`] arm each, with no wildcard: a sixth Pattern is a compile error
//! in [`subsumption`] rather than a silent inheritance of another family's
//! direction.
//!
//! # Reporting order within a family
//!
//! Selector roles are checked in declared-parameter order, then the evidence
//! condition, then retirement, and the first to fail is reported. A reviewer therefore always
//! sees the same reason for the same pair.
//!
//! # Why there is no `NotSubsumed`
//!
//! The analysis is conservative by construction: a false `Subsumed` silently
//! discards a Candidate that catches something no deployed Specification
//! catches, so `Subsumed` is returned only when containment is proven from the
//! two instantiations alone. That half needs no solver.
//!
//! The other half — proving containment *fails* — cannot be done here at all,
//! and a `bool` would be unsound in that direction too. **Selector divergence
//! does not imply non-subsumption, because `∅ ⊆ anything`.** If `C`'s trigger
//! demands `{region: "eu"}` and no Event in the deployment ever carries it,
//! `C`'s Violate-set is empty and `C` is subsumed by every Specification.
//! Symmetrically, a Candidate whose trigger demands nothing is structurally not
//! subsumed by one demanding `{region: "eu"}`, and is genuinely subsumed in a
//! deployment that only ever emits `region = "eu"`. Both turn on what a
//! deployment can emit, and ADR-0004 decided a declared Topology is just another
//! Specification checked against observation, never an input an analysis may
//! depend on. An SMT solver would add completeness, not soundness, and would
//! still need that input.
//!
//! So a failed argument is [`Undecided`]: *not proven, and not decided*. It is
//! kept separate from [`NotComparable`] because the named follow-on — searching
//! a recorded corpus for a Trace on which `C` violates and `S` does not, in
//! `sentinel-candidate` — can escalate an `Undecided` pair and can do nothing
//! with a `NotComparable` one. [`Subsumption::is_proven`] is the only boolean,
//! and it is named for what it means.
//!
//! # Two deployments, two Filters: why one abstract Trace is enough
//!
//! The semantics quantify over one shared `(Trace, Binding)`, but `C` and `S`
//! would run behind two Event Filters and two Evaluators. The lifting is sound:
//! [`FilterSource::dependencies`](crate::FilterSource::dependencies) projects
//! each selector to its kind and attribute **keys**, never its values; the
//! Filter admits by kind (`filter.rs`, S1) and retains every key either side
//! reads (S2); and inclusion has already forced kind equality on every role. No
//! Event one side's selectors or anchor sets read is dropped by the other
//! side's Filter.
//!
//! # No counterexample Trace
//!
//! Issue #177 asks for counterexamples, and this module produces none. A
//! counterexample is a `(Trace, Binding)` on which `C` settles `Violate` and `S`
//! does not; "settles `Violate`" is a judgement only the Verification Core makes
//! and `compile` cannot depend on `core` (the edge would be a cycle); and
//! synthesising a Trace would need to know what a deployment can emit, which is
//! the Topology again. What is delivered instead is a positive structural
//! witness for [`Subsumption::Subsumed`] ([`SubsumptionProof`]) and a
//! divergence report for [`Subsumption::Undecided`] — neither is called a
//! counterexample.
//!
//! # Grounded by execution
//!
//! Both family arguments are a hand-derived second statement of two Core
//! decision tables. `core/tests/subsumption_cross_check.rs` executes them
//! against the real tables — both CAT-1 `Violate` branches, and CAT-4 — and
//! holds a found Trace for each selector and required-source shape it refuses
//! (`RetirementDiffers` has no catalogued pair to replay): a Candidate that
//! narrows CAT-1's requirement violates where the deployed Specification does
//! not, which is why that pair is refused rather than cautiously refused.

#![allow(clippy::module_name_repetitions)] // `subsumption::Subsumption` names the analysis on purpose.
#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display, Formatter};

use sentinel_model::{SchemaVersion, SourceIdentity, SourceTier};

use crate::catalogue::{
    PatternId, PatternInstance, PRECEDENCE_REQUIREMENT, PRECEDENCE_TRIGGER, TAINT_ACTION,
    TAINT_UNTRUSTED_CONTENT,
};
use crate::ir::{PrecedenceIr, RetirementIr, SelectorDivergence, SelectorIr, TaintIr};
use crate::FilterIdentity;

/// The Candidate whose Violate-set is being asked about.
///
/// A newtype rather than a bare `&PatternInstance` because the two arguments of
/// [`subsumption`] are otherwise the same type, and transposing them inverts
/// the question. ADR-0031's `core/src/settlement.rs` case is this defect: two
/// adjacent same-typed parameters, and the whole suite green with them swapped.
#[derive(Clone, Copy, Debug)]
pub struct CandidateUnderReview<'instance>(&'instance PatternInstance);

impl<'instance> CandidateUnderReview<'instance> {
    /// Names this instance as the Candidate side of the question.
    #[must_use]
    pub const fn new(instance: &'instance PatternInstance) -> Self {
        Self(instance)
    }

    /// Returns the instance under review.
    #[must_use]
    pub const fn instance(&self) -> &'instance PatternInstance {
        self.0
    }
}

/// The deployed Specification whose Violate-set is the containing one.
///
/// One constructor and no rollout assertion. Whether this Specification is
/// warming behind an activation barrier changes its authority and never its
/// outcome, so there is nothing for a constructor to assert; whether it
/// declares cold guards or overrides its retirement are limits this module's
/// header states, which a constructor in `compile` cannot see either.
#[derive(Clone, Copy, Debug)]
pub struct DeployedSpecification<'instance>(&'instance PatternInstance);

impl<'instance> DeployedSpecification<'instance> {
    /// Names this instance as the deployed side of the question.
    #[must_use]
    pub const fn new(instance: &'instance PatternInstance) -> Self {
        Self(instance)
    }

    /// Returns the deployed instance.
    #[must_use]
    pub const fn instance(&self) -> &'instance PatternInstance {
        self.0
    }
}

/// Whether a Candidate's Violate-set is contained in a deployed Specification's.
///
/// Three-valued and deliberately missing a `NotSubsumed`: this analysis can
/// prove containment and can never refute it, because refuting it needs a
/// judgement about what a deployment can emit, and under ADR-0004 a declared
/// Topology is just another Specification, never an input an analysis may
/// depend on. A Candidate whose selectors diverge
/// from every deployed Specification may still have an empty Violate-set, and
/// the empty set is contained in everything. See the module header.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Subsumption {
    /// Proven. Every `(Trace, Binding)` on which the Candidate's monitor settles
    /// a `Violate` outcome is one on which the deployed Specification's monitor
    /// does too. See [`SubsumptionProof`] for what this does **not** say.
    Subsumed(SubsumptionProof),
    /// Not proven, and not decided. The answer may be either.
    Undecided(Undecided),
    /// The question is not well posed for this pair.
    NotComparable(NotComparable),
}

impl Subsumption {
    /// Returns whether containment is **proven**.
    ///
    /// The only value a caller may act on to refuse a Candidate as adding no
    /// Violate. `false` means *keep the Candidate*; it never means the
    /// Candidate was shown to add a Violate.
    #[must_use]
    pub const fn is_proven(&self) -> bool {
        matches!(self, Self::Subsumed(_))
    }
}

impl Display for Subsumption {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Subsumed(proof) => write!(formatter, "subsumed: {proof}"),
            Self::Undecided(undecided) => {
                write!(formatter, "not proven either way: {undecided}")
            }
            Self::NotComparable(reason) => write!(formatter, "not comparable: {reason}"),
        }
    }
}

/// What discharged a proven [`Subsumption::Subsumed`] — enough for a reviewer
/// to re-derive the proof by eye.
///
/// **Scope of the claim.** Outcome-level containment of the two IR-lowered
/// monitors' hot assessments. It does not say the deployed Specification's
/// *Verdict* is a `Violate` wherever the Candidate's would be: a Refused cold
/// guard on the deployed side rewrites that Violate to
/// `Inconclusive(EpistemicUncertainty)`, a retirement override takes the
/// deployed side outside the argument, a decisive witness from an unrequired
/// source fails its Verdict assembly, and authority is out of scope
/// altogether. The module header states all four.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SubsumptionProof {
    /// The Pattern both instances instantiate.
    pub pattern: PatternId,
    /// Identity of the deployed Specification the Candidate adds nothing to.
    pub deployed: FilterIdentity,
    /// One entry per selector role the Pattern declares, in declared-parameter
    /// order.
    pub inclusions: Vec<SelectorInclusion>,
    /// The evidence condition the Pattern demands, which held.
    pub evidence: EvidenceInclusion,
}

impl Display for SubsumptionProof {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} adds no Violate outcome that {} {} does not already reach (",
            self.pattern, self.deployed.specification_id, self.deployed.specification_version
        )?;
        for inclusion in &self.inclusions {
            write!(formatter, "{inclusion}; ")?;
        }
        write!(formatter, "{})", self.evidence)
    }
}

/// One selector role's discharged inclusion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SelectorInclusion {
    /// The declared Pattern parameter that bound the selector
    /// ([`PRECEDENCE_TRIGGER`], [`TAINT_UNTRUSTED_CONTENT`], …).
    pub parameter: &'static str,
    /// Which way inclusion had to run for this role, and did.
    pub direction: InclusionDirection,
}

impl Display for SelectorInclusion {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.parameter, self.direction)
    }
}

/// Which side of a selector role must match no more Events than the other.
///
/// Carried rather than implied, because the direction is not uniform within a
/// Pattern: CAT-1's `requirement` discharges an obligation and runs the
/// opposite way from its `trigger`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InclusionDirection {
    /// The Candidate's selector must match no more Events than the deployed
    /// one. Obligation-raising and falsifying roles.
    CandidateNarrower,
    /// The deployed selector must match no more Events than the Candidate's.
    /// Obligation-**discharging** roles, where a narrower selector violates
    /// more rather than less.
    DeployedNarrower,
}

impl Display for InclusionDirection {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CandidateNarrower => "the Candidate's selector matches no more Events",
            Self::DeployedNarrower => {
                "the deployed Specification's selector matches no more Events"
            }
        })
    }
}

/// The condition a Pattern demands of the two required-source identity sets.
///
/// Names the condition **demanded and discharged**, so a reviewer can re-derive
/// the evidence half of a proof by eye. Identities only — minimum tiers are
/// unread on every decision path.
///
/// One variant today, and `#[non_exhaustive]` so a weaker condition is
/// additive. A `DeployedSubset` for CAT-4 was planned and is **not** here:
/// `core/tests/subsumption_cross_check.rs` executed it and found the deployed
/// Specification's monitor reaching the Violate while its Verdict assembly
/// refused the witness from the dropped source (this module's header).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EvidenceInclusion {
    /// The two identity sets must be equal.
    ///
    /// CAT-1 because its evidence gate and its silent-anchor condition move in
    /// opposite directions as the set grows; CAT-4 because Verdict assembly
    /// accepts a decisive witness only from a required source, so any
    /// difference changes which witnesses one side can cite.
    Equal,
}

impl Display for EvidenceInclusion {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Equal => "required sources: equal identity sets",
        })
    }
}

/// Why the structural argument did not close. Never a claim of
/// non-containment.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Undecided {
    /// A selector role is not ordered by inclusion in the direction this
    /// Pattern's polarity demands.
    SelectorNotIncluded {
        /// The declared parameter that bound the selector.
        parameter: &'static str,
        /// The direction the role needed.
        direction: InclusionDirection,
        /// The first reason inclusion failed, in the selector language's own
        /// subject/target terms. `Display` resolves which side is which.
        divergence: SelectorDivergence,
    },
    /// The two required-source identity sets do not meet the Pattern's
    /// evidence condition.
    ///
    /// What a difference does to a Violate is a deployment fact, and in
    /// neither derived family is it safe in either direction: CAT-1's gate and
    /// silent-anchor condition pull opposite ways, and CAT-4's Verdict
    /// assembly refuses a witness from a source the deployed side dropped.
    RequiredSourcesDiffer {
        /// The condition the Pattern demands, which failed.
        condition: EvidenceInclusion,
        /// Source identities only the Candidate requires.
        candidate_only: BTreeSet<SourceIdentity>,
        /// Source identities only the deployed Specification requires.
        deployed_only: BTreeSet<SourceIdentity>,
    },
    /// This analysis derives no inclusion argument for this Pattern yet.
    ///
    /// Not a defect in the pair. CAT-2, CAT-3 and CAT-5 each carry an
    /// interval, a Scope close or a numeric bound whose monotonicity lives in
    /// that family's Core decision table; deriving it here from the IR alone
    /// would be a second copy of that table (ADR-0027, issue #112).
    PolarityNotDerived {
        /// The Pattern both instances instantiate.
        pattern: PatternId,
    },
    /// The two sides declare different retirement.
    ///
    /// **Defensive: no public input reaches it today.** The catalogue lowers
    /// exactly one retirement per Pattern, this analysis compares same-Pattern
    /// pairs only, and both derived families declare
    /// [`RetirementIr::NotRequired`]. It is compared anyway so that a Pattern
    /// whose retirement becomes a parameter cannot be proven by an argument
    /// that never read it: retirement fixes a Binding's outcome when its Scope
    /// retires, and no inclusion argument across two retirements is derived.
    /// Reported last, after every selector role and the evidence condition.
    RetirementDiffers {
        /// The Candidate's retirement.
        candidate: RetirementIr,
        /// The deployed Specification's retirement.
        deployed: RetirementIr,
    },
}

impl Display for Undecided {
    /// Renders the reason with the Candidate and the deployed Specification
    /// named, resolving [`SelectorDivergence`]'s neutral subject/target.
    ///
    /// `AttributeValue` and `RetirementIr` have no `Display`, so they render
    /// with `{:?}`. `RetirementDiffers` is a defensive arm that no catalogued
    /// pair reaches today, not a live diagnostic.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::SelectorNotIncluded {
                parameter,
                direction,
                divergence,
            } => {
                let (subject, target) = match direction {
                    InclusionDirection::CandidateNarrower => {
                        ("the Candidate's", "the deployed Specification's")
                    }
                    InclusionDirection::DeployedNarrower => {
                        ("the deployed Specification's", "the Candidate's")
                    }
                };
                write!(
                    formatter,
                    "parameter {parameter}: {subject} selector must match no more Events than \
                     {target}, but "
                )?;
                match divergence {
                    SelectorDivergence::KindsDiffer {
                        subject: subject_kind,
                        target: target_kind,
                    } => write!(
                        formatter,
                        "{subject} selects Event kind {subject_kind} and {target} selects \
                         {target_kind}"
                    ),
                    SelectorDivergence::RequirementNotCarried { key, demanded } => write!(
                        formatter,
                        "{target} demands {key} = {demanded:?}, which {subject} does not"
                    ),
                    SelectorDivergence::RequirementsConflict {
                        key,
                        subject: subject_value,
                        target: target_value,
                    } => write!(
                        formatter,
                        "{subject} demands {key} = {subject_value:?} and {target} demands \
                         {key} = {target_value:?}"
                    ),
                }
            }
            Self::RequiredSourcesDiffer {
                condition,
                candidate_only,
                deployed_only,
            } => {
                let demanded = match condition {
                    EvidenceInclusion::Equal => "equal required-source identity sets",
                };
                write!(
                    formatter,
                    "the Pattern needs {demanded}; only the Candidate requires [{}], only the \
                     deployed Specification requires [{}]",
                    sources(candidate_only),
                    sources(deployed_only)
                )
            }
            Self::PolarityNotDerived { pattern } => write!(
                formatter,
                "no inclusion argument is derived for {pattern}; its Core decision table has a \
                 dimension this analysis does not restate"
            ),
            Self::RetirementDiffers {
                candidate,
                deployed,
            } => write!(
                formatter,
                "the Candidate declares retirement {candidate:?} and the deployed Specification \
                 {deployed:?}; no inclusion argument across two retirements is derived"
            ),
        }
    }
}

/// Why the question is not well posed for this pair.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum NotComparable {
    /// Different Patterns. Opposite polarity is not inclusion in either
    /// direction, and two Patterns in one family do not share an argument.
    DifferentPattern {
        /// The Candidate's Pattern.
        candidate: PatternId,
        /// The deployed Specification's Pattern.
        deployed: PatternId,
    },
    /// Different Binding attributes, so a Binding of one is not a Binding of
    /// the other and the two Violate-sets hold different kinds of element.
    DifferentBindingAttribute {
        /// The Candidate's Binding attribute.
        candidate: String,
        /// The deployed Specification's Binding attribute.
        deployed: String,
    },
    /// Different producer contracts, so one Event kind or attribute key need
    /// not denote the same thing on both sides.
    DifferentProducerContract {
        /// The Candidate's schema version.
        candidate: SchemaVersion,
        /// The deployed Specification's schema version.
        deployed: SchemaVersion,
    },
}

impl Display for NotComparable {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DifferentPattern {
                candidate,
                deployed,
            } => write!(
                formatter,
                "the Candidate instantiates {candidate} and the deployed Specification \
                 {deployed}"
            ),
            Self::DifferentBindingAttribute {
                candidate,
                deployed,
            } => write!(
                formatter,
                "the Candidate binds under {candidate} and the deployed Specification under \
                 {deployed}"
            ),
            Self::DifferentProducerContract {
                candidate,
                deployed,
            } => write!(
                formatter,
                "the Candidate is written against producer contract {candidate} and the \
                 deployed Specification against {deployed}"
            ),
        }
    }
}

/// Asks whether the Candidate's Violate-set is contained in the deployed
/// Specification's.
///
/// Conservative by construction: [`Subsumption::Subsumed`] is returned only
/// when containment is proven from the two instantiations alone, with no appeal
/// to any fact about a deployment. Every other answer means *keep the
/// Candidate*. Read the module header for the semantics, the per-family
/// directions, and the Core-side limits — cold guards, a retirement override,
/// unrequired witness sources — that the caller owns.
///
/// Exhaustive over [`PatternId`] with no wildcard, so a Pattern added to the
/// catalogue without a derived argument is a compile error here rather than a
/// silent inheritance of another family's direction — the forcing function
/// [`PatternInstance::exercise`] uses.
#[must_use]
pub fn subsumption(
    candidate: CandidateUnderReview<'_>,
    deployed: DeployedSpecification<'_>,
) -> Subsumption {
    let (candidate, deployed) = (candidate.0, deployed.0);
    if let Some(reason) = not_comparable(candidate, deployed) {
        return Subsumption::NotComparable(reason);
    }

    // `PatternId` equality holds from here on, so the match is on it — five
    // arms, no wildcard — and the `unreachable!`s guard an impossible *pairing*
    // inside one arm, never the enum's growth. `lower` pairs each `PatternId`
    // with exactly one `MonitorIr` variant, pinned by
    // `every_pattern_lowers_to_the_monitor_its_family_needs`.
    let pattern = candidate.pattern();
    let argued = match pattern {
        PatternId::PrecedenceApproval => {
            match (
                candidate.ir().as_precedence(),
                deployed.ir().as_precedence(),
            ) {
                (Some(candidate_ir), Some(deployed_ir)) => {
                    precedence_inclusions(candidate_ir, deployed_ir)
                }
                _ => unreachable!("CAT-1 lowers to MonitorIr::Precedence"),
            }
        }
        PatternId::ProvenanceTaint => match (candidate.ir().as_taint(), deployed.ir().as_taint()) {
            (Some(candidate_ir), Some(deployed_ir)) => taint_inclusions(candidate_ir, deployed_ir),
            _ => unreachable!("CAT-4 lowers to MonitorIr::Taint"),
        },
        // One arm, three named variants, still no wildcard: a family whose
        // argument is derived splits out of this group, and a sixth Pattern is
        // a compile error here.
        PatternId::AbsenceAfterUntil
        | PatternId::AggregationCountWithin
        | PatternId::ExistenceBeforeScopeClose => Err(Undecided::PolarityNotDerived { pattern }),
    };

    match argued {
        Ok(Argued {
            inclusions,
            evidence,
        }) => Subsumption::Subsumed(SubsumptionProof {
            pattern,
            deployed: deployed.identity().clone(),
            inclusions,
            evidence,
        }),
        Err(undecided) => Subsumption::Undecided(undecided),
    }
}

/// The three well-posedness preconditions, in their documented order.
fn not_comparable(
    candidate: &PatternInstance,
    deployed: &PatternInstance,
) -> Option<NotComparable> {
    if candidate.pattern() != deployed.pattern() {
        return Some(NotComparable::DifferentPattern {
            candidate: candidate.pattern(),
            deployed: deployed.pattern(),
        });
    }
    let (candidate_binding, deployed_binding) = (
        candidate.ir().binding_attribute(),
        deployed.ir().binding_attribute(),
    );
    if candidate_binding != deployed_binding {
        return Some(NotComparable::DifferentBindingAttribute {
            candidate: candidate_binding.clone(),
            deployed: deployed_binding.clone(),
        });
    }
    let (candidate_contract, deployed_contract) = (
        candidate.contract().schema_version(),
        deployed.contract().schema_version(),
    );
    if candidate_contract != deployed_contract {
        return Some(NotComparable::DifferentProducerContract {
            candidate: candidate_contract.clone(),
            deployed: deployed_contract.clone(),
        });
    }
    None
}

/// A family argument that closed: what the proof records.
struct Argued {
    inclusions: Vec<SelectorInclusion>,
    evidence: EvidenceInclusion,
}

/// CAT-1: trigger Candidate-narrower, requirement **deployed**-narrower,
/// required-source identities equal, retirement equal — checked in that order.
fn precedence_inclusions(
    candidate: &PrecedenceIr,
    deployed: &PrecedenceIr,
) -> Result<Argued, Undecided> {
    // No `..` on either side: a field added to `PrecedenceIr` fails to compile
    // here (E0027), where its effect on the Violate-set has to be argued,
    // instead of being silently ignored by a proof (ADR-0031).
    let PrecedenceIr {
        specification_id: _,                 // identity only: changes no outcome
        specification_version: _,            // identity only: changes no outcome
        schema_version: _,                   // checked in the prologue (`NotComparable`)
        binding_attribute: _,                // checked in the prologue (`NotComparable`)
        trigger: candidate_trigger,          // compared: Candidate narrower
        requirement: candidate_requirement,  // compared: deployed narrower
        required_sources: candidate_sources, // compared: identity sets equal
        retirement: candidate_retirement,    // compared: equal, else `Undecided`
    } = candidate;
    let PrecedenceIr {
        specification_id: _,
        specification_version: _,
        schema_version: _,
        binding_attribute: _,
        trigger: deployed_trigger,
        requirement: deployed_requirement,
        required_sources: deployed_sources,
        retirement: deployed_retirement,
    } = deployed;

    let inclusions = vec![
        role(
            PRECEDENCE_TRIGGER,
            InclusionDirection::CandidateNarrower,
            candidate_trigger,
            deployed_trigger,
        )?,
        role(
            PRECEDENCE_REQUIREMENT,
            InclusionDirection::DeployedNarrower,
            candidate_requirement,
            deployed_requirement,
        )?,
    ];
    let evidence = evidence_condition(
        EvidenceInclusion::Equal,
        candidate_sources,
        deployed_sources,
    )?;
    same_retirement(*candidate_retirement, *deployed_retirement)?;
    Ok(Argued {
        inclusions,
        evidence,
    })
}

/// CAT-4: action and untrusted-content both Candidate-narrower, required-source
/// identities equal, retirement equal — in that order.
fn taint_inclusions(candidate: &TaintIr, deployed: &TaintIr) -> Result<Argued, Undecided> {
    // No `..` on either side, for `precedence_inclusions`' reason.
    let TaintIr {
        specification_id: _,                  // identity only: changes no outcome
        specification_version: _,             // identity only: changes no outcome
        schema_version: _,                    // checked in the prologue (`NotComparable`)
        binding_attribute: _,                 // checked in the prologue (`NotComparable`)
        action: candidate_action,             // compared: Candidate narrower
        untrusted_content: candidate_content, // compared: Candidate narrower
        required_sources: candidate_sources,  // compared: identity sets equal
        retirement: candidate_retirement,     // compared: equal, else `Undecided`
    } = candidate;
    let TaintIr {
        specification_id: _,
        specification_version: _,
        schema_version: _,
        binding_attribute: _,
        action: deployed_action,
        untrusted_content: deployed_content,
        required_sources: deployed_sources,
        retirement: deployed_retirement,
    } = deployed;

    let inclusions = vec![
        role(
            TAINT_ACTION,
            InclusionDirection::CandidateNarrower,
            candidate_action,
            deployed_action,
        )?,
        role(
            TAINT_UNTRUSTED_CONTENT,
            InclusionDirection::CandidateNarrower,
            candidate_content,
            deployed_content,
        )?,
    ];
    let evidence = evidence_condition(
        EvidenceInclusion::Equal,
        candidate_sources,
        deployed_sources,
    )?;
    same_retirement(*candidate_retirement, *deployed_retirement)?;
    Ok(Argued {
        inclusions,
        evidence,
    })
}

/// Checks that both sides declare the same retirement, after every selector
/// role and the evidence condition.
///
/// Defensive: see [`Undecided::RetirementDiffers`] for why no catalogued pair
/// reaches it today and why it is compared anyway.
fn same_retirement(candidate: RetirementIr, deployed: RetirementIr) -> Result<(), Undecided> {
    if candidate == deployed {
        return Ok(());
    }
    Err(Undecided::RetirementDiffers {
        candidate,
        deployed,
    })
}

/// Checks one selector role in the direction its polarity demands.
///
/// One function rather than a family-side call plus a separately written
/// [`InclusionDirection`]: the direction **reported** and the check
/// **performed** are derived from the same argument, so a family argument
/// cannot report `DeployedNarrower` and test the other way round. That
/// transposition is the contravariance bug this module is arranged around,
/// reachable through the reporting path — ADR-0031's settlement case in
/// miniature.
fn role(
    parameter: &'static str,
    direction: InclusionDirection,
    candidate: &SelectorIr,
    deployed: &SelectorIr,
) -> Result<SelectorInclusion, Undecided> {
    let (subject, target) = match direction {
        InclusionDirection::CandidateNarrower => (candidate, deployed),
        InclusionDirection::DeployedNarrower => (deployed, candidate),
    };
    subject.narrowing_divergence(target).map_or(
        Ok(SelectorInclusion {
            parameter,
            direction,
        }),
        |divergence| {
            Err(Undecided::SelectorNotIncluded {
                parameter,
                direction,
                divergence,
            })
        },
    )
}

/// Checks the required-source identity sets against the condition a Pattern
/// demands — the evidence twin of [`role`], for the same reason: the reported
/// condition and the performed comparison are one value.
fn evidence_condition(
    condition: EvidenceInclusion,
    candidate: &BTreeMap<SourceIdentity, SourceTier>,
    deployed: &BTreeMap<SourceIdentity, SourceTier>,
) -> Result<EvidenceInclusion, Undecided> {
    let candidate: BTreeSet<&SourceIdentity> = candidate.keys().collect();
    let deployed: BTreeSet<&SourceIdentity> = deployed.keys().collect();
    let holds = match condition {
        EvidenceInclusion::Equal => candidate == deployed,
    };
    if holds {
        return Ok(condition);
    }
    Err(Undecided::RequiredSourcesDiffer {
        condition,
        candidate_only: candidate
            .difference(&deployed)
            .map(|source| (*source).clone())
            .collect(),
        deployed_only: deployed
            .difference(&candidate)
            .map(|source| (*source).clone())
            .collect(),
    })
}

/// Renders a set of source identities for a reviewer, in set order.
fn sources(set: &BTreeSet<SourceIdentity>) -> String {
    set.iter()
        .map(|source| format!("{}/{}", source.monitored_application(), source.source()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod retirement {
    //! `Undecided::RetirementDiffers`, tested where it can be reached.
    //!
    //! No public input reaches the variant: `PatternInstance`'s IR is private,
    //! and the catalogue lowers one retirement per Pattern. So these tests build
    //! the IR directly and call the family arguments directly. Without them, a
    //! retirement comparison that always passed would leave every test green.

    use std::collections::BTreeMap;

    use sentinel_model::{
        EventKind, MonitoredApplicationId, SchemaVersion, SourceId, SourceIdentity, SourceTier,
    };

    use super::{
        precedence_inclusions, taint_inclusions, PatternId, PrecedenceIr, RetirementIr, SelectorIr,
        TaintIr, Undecided,
    };

    const GATEWAY: &str = "example-gateway";
    const REPLICA: &str = "example-authz-replica";

    fn selector(kind: &str) -> SelectorIr {
        SelectorIr::new(EventKind::new(kind).expect("constant"), BTreeMap::new())
    }

    fn required(application: &str) -> BTreeMap<SourceIdentity, SourceTier> {
        BTreeMap::from([(
            SourceIdentity::new(
                MonitoredApplicationId::new(application).expect("constant"),
                SourceId::new("default").expect("constant"),
            ),
            SourceTier::Anchor,
        )])
    }

    fn precedence(retirement: RetirementIr, application: &str) -> PrecedenceIr {
        PrecedenceIr {
            specification_id: "example.precedence".to_owned(),
            specification_version: "v1".to_owned(),
            schema_version: SchemaVersion::new("example-contract-v1").expect("constant"),
            binding_attribute: "example.request".to_owned(),
            trigger: selector("example.action.executed"),
            requirement: selector("example.approval.granted"),
            required_sources: required(application),
            retirement,
        }
    }

    fn taint(retirement: RetirementIr, application: &str) -> TaintIr {
        TaintIr {
            specification_id: "example.taint".to_owned(),
            specification_version: "v1".to_owned(),
            schema_version: SchemaVersion::new("example-contract-v1").expect("constant"),
            binding_attribute: "example.request".to_owned(),
            action: selector("example.mail.sent"),
            untrusted_content: selector("example.content.ingested"),
            required_sources: required(application),
            retirement,
        }
    }

    #[test]
    fn a_retirement_difference_is_undecided_in_both_derived_families() {
        // Equal retirement: the argument closes, so retirement is the only
        // cause of the two refusals below.
        assert!(precedence_inclusions(
            &precedence(RetirementIr::NotRequired, GATEWAY),
            &precedence(RetirementIr::NotRequired, GATEWAY),
        )
        .is_ok());
        assert!(taint_inclusions(
            &taint(RetirementIr::NotRequired, GATEWAY),
            &taint(RetirementIr::NotRequired, GATEWAY),
        )
        .is_ok());

        assert_eq!(
            precedence_inclusions(
                &precedence(RetirementIr::NotRequired, GATEWAY),
                &precedence(RetirementIr::Deadline, GATEWAY),
            )
            .err(),
            Some(Undecided::RetirementDiffers {
                candidate: RetirementIr::NotRequired,
                deployed: RetirementIr::Deadline,
            })
        );
        assert_eq!(
            taint_inclusions(
                &taint(RetirementIr::Inactivity, GATEWAY),
                &taint(RetirementIr::NotRequired, GATEWAY),
            )
            .err(),
            Some(Undecided::RetirementDiffers {
                candidate: RetirementIr::Inactivity,
                deployed: RetirementIr::NotRequired,
            })
        );
    }

    #[test]
    fn retirement_is_reported_after_the_selector_roles_and_the_evidence_condition() {
        // The evidence condition and retirement both fail: evidence is reported.
        assert!(matches!(
            precedence_inclusions(
                &precedence(RetirementIr::NotRequired, GATEWAY),
                &precedence(RetirementIr::Deadline, REPLICA),
            )
            .err(),
            Some(Undecided::RequiredSourcesDiffer { .. })
        ));

        // A selector role and retirement both fail: the role is reported.
        let mut deployed = taint(RetirementIr::Deadline, GATEWAY);
        deployed.action = selector("example.mail.drafted");
        assert!(matches!(
            taint_inclusions(&taint(RetirementIr::NotRequired, GATEWAY), &deployed).err(),
            Some(Undecided::SelectorNotIncluded {
                parameter: "action",
                ..
            })
        ));
    }

    #[test]
    fn the_retirement_difference_names_which_side_is_the_candidate() {
        let undecided = Undecided::RetirementDiffers {
            candidate: RetirementIr::NotRequired,
            deployed: RetirementIr::Deadline,
        };
        assert_eq!(
            undecided.to_string(),
            "the Candidate declares retirement NotRequired and the deployed Specification \
             Deadline; no inclusion argument across two retirements is derived"
        );
    }

    /// Every `Undecided` variant, matched with no wildcard. `Undecided` is
    /// `#[non_exhaustive]` outside this crate, so the pin lives here: a new
    /// variant fails to compile in this test, beside the one that pins its
    /// place in the reporting order.
    #[test]
    fn undecided_has_exactly_these_variants() {
        fn name(undecided: &Undecided) -> &'static str {
            match undecided {
                Undecided::SelectorNotIncluded { .. } => "selector role",
                Undecided::RequiredSourcesDiffer { .. } => "evidence condition",
                Undecided::PolarityNotDerived { .. } => "polarity not derived",
                Undecided::RetirementDiffers { .. } => "retirement",
            }
        }
        assert_eq!(
            name(&Undecided::PolarityNotDerived {
                pattern: PatternId::AbsenceAfterUntil,
            }),
            "polarity not derived"
        );
        assert_eq!(
            name(&Undecided::RetirementDiffers {
                candidate: RetirementIr::NotRequired,
                deployed: RetirementIr::Deadline,
            }),
            "retirement"
        );
    }
}
