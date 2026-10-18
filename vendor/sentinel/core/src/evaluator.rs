//! The resumable stepper: `Input` in, `Settled | Pending | NeedsFacts` out.
//!
//! # Why `Input` is an enum and not just an Event
//!
//! ADR-0012 says "stepping it with an Event". But settlement is Watermark
//! driven, and the evidence gate genuinely reads completeness signals, gaps and
//! observed-source membership. In a resumable Core those can come from neither
//! a clock nor a store, so per-source evidence has to be an **input**.
//! [`Input::Source`] is the only shape that keeps "no I/O, no clock" true.
//!
//! # The emission rule, and why "at most twice" is a theorem
//!
//! A Binding is emitted in [`Step::Settled`] when **(i)** its hot monitor is
//! settled at the current evidence state — the gate is open and
//! `watermarks_cover_epochs()` is true — and **(ii)** its Verdict bytes differ
//! from the last emission for that Binding. Outstanding cold guards do **not**
//! suppress emission; they set `cold_path = PENDING` and
//! `authority = Provisional`. There is deliberately no error for stepping while
//! queries are outstanding: outstanding queries are the designed steady state
//! (the cold path lags), and refusing ingestion while they are open would block
//! the hot path on the cold path — the latency coupling ADR-0005 and ADR-0012
//! exist to forbid. Enforcement sits entirely on the transition **into**
//! `Authoritative`, never on ingestion.
//!
//! From `crate::evidence`'s contract it follows that:
//!
//! > Under (a) a source's epoch cannot reopen and its tier, capabilities and
//! > gaps cannot be restated; under (b) no Event may arrive
//! > above a closed boundary; under (c) the gate cannot open until the Core
//! > holds every slot up to it. Therefore once a Binding's hot monitor settles,
//! > no admissible input can change its outcome *of the Violate class*. The
//! > fields that can subsequently differ are `authority`, `cold_path` and
//! > `cold_settlement_watermarks` — a `Provisional` emission at hot settlement,
//! > then an `Authoritative` one when every guard settles (ADR-0021: "the hot
//! > verdict was never authoritative, so there is nothing to retract"). A
//! > terminal [`crate::facts::GuardConclusion::Refused`] also settles, and
//! > that settlement may rewrite the outcome `Satisfy → Inconclusive`
//! > (epistemic). That rewrite is this issue's envelope, not a `Violate`
//! > flip; limitation 8 and #20 still forbid cold `Violate` over hot
//! > `Satisfy`. Hence **at most two emissions per Binding**.
//!
//! That derivation also leans on a step worth stating: **gate-open implies
//! coverage complete.** The gate fires G1 unless every required source appears
//! in the observed set, and coverage completeness (`required` is a subset of
//! the ledger's observed sources) is exactly that predicate.
//! So a first emission always occurs at complete coverage, and `Provisional`
//! there can only mean unsettled guards — never missing sources.
//!
//! The reasoning is **scoped to a single epoch by construction**. An epoch
//! rollover can legitimately deliver a witness that changes an already-settled
//! Binding, and #12 has no fixture, no partner evidence and no ADR-0019 licence
//! to design multi-epoch behaviour. That is disclosed as a scope limit rather
//! than papered over.
//!
//! # Scope retirement: Steps 4, 5 and 6 (#110, ADR-0021's O-03)
//!
//! Steps 1–3 rest on one premise: **gate-open implies coverage complete**, so
//! nothing is ever emitted behind a closed gate. Scope retirement is the first
//! thing in this Core that emits behind one, so the theorem is **amended**
//! here rather than extended by corollary.
//!
//! A *retirement signal* is the periphery's statement that it has stopped
//! waiting for a Scope to close. It carries a **Scope horizon** on the Sentinel
//! observation-time axis, and the Core **never measures it**: the horizon
//! arrives as an input, exactly as `Input::Source` carries completeness, and
//! the Core only ever compares it against each Event's own
//! `sentinel_model::Event::observation_time`. Admissibility is therefore a
//! function of observation time and **never of arrival position**, which is
//! what keeps a Verdict a function of the input *multiset*. A horizon on a
//! different `SentinelClockId` yields no conclusion at all, mirroring
//! `sentinel_model::CompletenessFreshness::accepts` returning `None`.
//!
//! Write `H(b)` for one Binding's lifecycle — `Open`, or retired at a horizon —
//! and `Slice(b)` for the admitted member set when `H(b)` is `Open`, narrowed
//! to the members at or below the horizon when it is retired. `Slice(b)` is
//! **recomputed at assessment time, never mutated at admission time**.
//!
//! ## Step 4 — retirement is an emission licence *and* an outcome pin
//!
//! A Binding is emitted in [`Step::Settled`] when
//!
//! > **(i)** `gate.is_open() && gate.watermarks_cover_epochs()`, **or** `H(b)`
//! > is not `Open` **and** Evidence Coverage is non-empty; and **(ii)** its
//! > emission key differs from the last emission for that Binding.
//!
//! The retired disjunct is **per Binding**, and is applied where the Binding is
//! bound rather than on the global branch: a Binding that has not retired is
//! not licensed by a sibling's retirement. Widening the global branch instead
//! would emit every `Open` Binding behind a closed gate and destroy Steps 1–3's
//! premise outright.
//!
//! The Evidence-Coverage conjunct is soundness-critical **for liveness, not for
//! safety**. `refuse_unconstructible_envelope` returns
//! [`crate::CoreError::NoEvidenceCoverage`] when at least one assessment exists
//! and no required source is covered at all; without the conjunct, a retirement
//! recorded before any `Input::Source` would make every subsequent step return
//! that error — with the lifecycle mutation already committed — and the
//! evaluator would be wedged.
//!
//! `declare_queries` is deliberately **not** widened alongside it, and stays
//! gate-open only. `crate::facts::FactLedger::declare` is idempotent by
//! `(binding, guard)`, so the floor is snapshotted into a
//! `crate::facts::FactQuery` once and never refreshed; declaring behind a
//! closed gate would permanently install a *lower* floor and let a cold answer
//! settle a conjunct it never saw. The cost is disclosed rather than hidden: a
//! retired guard-declaring Specification never upgrades. #18 owns the
//! retirement horizon for the cold path.
//!
//! When `H(b)` is retired the Verdict's **outcome** is
//! `sentinel_model::VerdictOutcome::Inconclusive` unconditionally — never
//! `Violate`, never `Satisfy`, and *structurally* so. The retired build path
//! runs through `crate::precedence::RetiredAssessment`, which carries no
//! outcome field at all, so neither value is constructible on that path rather
//! than merely demoted from it. That is ADR-0031 clause 1's shape, the same one
//! `crate::taint::ConvictedOutcome` already uses. The **reason** is
//! `sentinel_model::InconclusiveReason::PrefixUncertainty` **unless** a
//! terminal `crate::facts::GuardConclusion::Refused` rewrites it to
//! `EpistemicUncertainty`; that rewrite is licensed and is not an outcome
//! change, so nothing may claim a retired Binding's reason is unconditional.
//!
//! *Liveness, stated precisely.* A retired Binding emits on the next
//! `advance()` at which **(a)** at least one required source is covered and
//! **(b)** no causal contradiction is outstanding — `advance` returns
//! [`Step::Pending`] before the gate is evaluated while
//! [`Evaluator::causal_contradiction`] is `Some`, and [`Evaluator::seal`] then
//! raises [`crate::CoreError::Causality`] rather than the retired Verdict. If
//! no required source *ever* declares, the retired Binding never emits
//! mid-stream and `seal()` returns [`crate::CoreError::NoEvidenceCoverage`],
//! which is the rule that already governs a non-retired evaluator. It fails
//! **loudly**; there is no silent vanish. Both caveats are liveness only, and
//! the safe direction is unaffected.
//!
//! ## Step 5 — the retired state is absorbing, and "at most two" survives
//!
//! It does not survive on the envelope key. [`crate::settlement`] derives
//! `cold_path` from coverage and guards and `authority` from `cold_path` *and*
//! the activation barrier, and those move independently: for a guard-free
//! Specification whose coverage completes before its barrier clears,
//! `(authority, cold_path)` takes **three** values in sequence, and a Binding
//! emitting behind a closed gate sees all three. A non-retired Binding is
//! immune only because gate-open implies complete coverage.
//!
//! The repair is a strictly monotone **one-bit** key on the retired arm. A
//! retired Binding's outcome is a constant, so the only decisive content left
//! is whether authority has reached `Authoritative`: one bit, which advances at
//! most once and is never written back. A `(reason, authoritative)` key would
//! give three emissions for two guards where only one refuses — an independent
//! second reason the one-bit shape is right.
//!
//! > **Step 5.** By clause (d2) below, a Binding that is ever retired has made
//! > zero emissions at the moment of retirement. Under Step 4 its outcome is
//! > thereafter a constant, so its key carries exactly one bit, which advances
//! > at most once and never regresses by construction. A retired Binding
//! > therefore emits at most twice: once on the first `advance()` satisfying
//! > Step 4's liveness conditions, and once more iff its authority reaches
//! > `Authoritative`. A Binding that is never retired is governed by Steps 1–3
//! > unchanged. **Hence at most two emissions per Binding.**
//!
//! Declining to re-emit on an `Authoritative → Provisional` regression is
//! correct independently: `sentinel-store` refuses exactly that transition.
//! Limitation 12 in [`crate`] says the restart is harmless today *because the
//! gate closes and `advance` re-emits nothing*; retirement removes that
//! protection, and the sticky bit is what replaces it.
//!
//! The last emitted envelope for a retired Binding may be **less complete than
//! the Core's final knowledge**. [`Evaluator::seal`] emits the final one, with
//! no dedup, so nothing is lost at the end of an evaluation.
//!
//! ## Step 6 — the admissibility contract, clause (d)
//!
//! Symmetric to `crate::evidence`'s clauses (a), (b) and (c): a breach is a
//! [`crate::CoreError`], not a degrade, and **every clause is validated before
//! any state is mutated**, so a rejected signal leaves the evaluator usable —
//! an empty `resume()` still succeeds and `seal()` still returns Verdicts. The
//! order is load-bearing:
//!
//! 1. **(d0) declaration.** The Specification's declared retirement policy is
//!    not `NotRequired` **and** the signal's cause is the one it declares.
//!    Order-free: a property of the Specification alone. This is also how CAT-4
//!    is excluded — its IR declares `NotRequired`, read through the same
//!    accessor, so retiring a CAT-4 Binding is inadmissible *because the
//!    catalogue says so* rather than because the Core hardcodes a family check.
//! 2. **(d3) the join, checked _before_ (d2).** Horizons **join upward**; they
//!    are not frozen at entry. Comparing an incoming horizon with the current
//!    one is four-valued, because comparison is defined only within one
//!    `SentinelClockId`: *same* is an idempotent no-op, *above* advances the
//!    join with no re-emission (the key is unchanged), *below* is rejected as a
//!    regression, and a *different Sentinel clock* is rejected as incomparable.
//!    (d3) must precede (d2) or idempotence would be unreachable, since a
//!    retired Binding has already emitted by the time a second signal arrives.
//! 3. **(d2) no retirement of an already-emitted Binding**, on the
//!    `Open → Retired` transition only. This is what buys back "at most two";
//!    absorbing-ness alone does not.
//!
//! There is deliberately **no** clause rejecting a horizon that sits below an
//! already-delivered observation. That is arrival-position reasoning, the exact
//! thing this design forbids, and it buys nothing the outcome pin does not
//! already give. Its diagnostic value is kept as a disclosure computed at
//! *assessment* time — how many of the Binding's members sit above the horizon
//! — because at assessment time every input has arrived, so the count is a
//! function of the multiset. Computing it at retirement time would be
//! order-sensitive and would land inside the encoded Verdict.
//!
//! Recording a lifecycle for a Binding the Core has never admitted an Event for
//! is **admissible and produces no Verdict**: assessment iterates the Slice
//! index's Bindings, and an unseen Binding is not among them. Rejecting it
//! would be order-sensitive on `{Retire(b), e_of_b}`, and manufacturing an
//! assessment for it would re-open the wedge Step 4's coverage conjunct closes.
//!
//! **The residue, stated rather than discovered.** For two retirement signals
//! on *different* Sentinel clocks the rejection is symmetric — both orders
//! error on the second signal — but the retained state is not: whichever
//! arrived first is the one kept. The same holds for a rejected regression. The
//! error is recoverable and does not wedge, so a driver that ignores it and
//! seals anyway reads a different `Slice(b)` in the two orders: the issue's own
//! defect-3 shape, reached **after a recoverable error**, and therefore outside
//! a determinism claim quantified over *admissible* sequences. It is also why
//! horizons join rather than freeze — the join removes the residue outright for
//! the single-clock case, which is every admissible case.
//!
//! # `seal()`
//!
//! [`Evaluator::seal`] is the driver's statement that no further [`Input`] will
//! arrive, and it emits the current Verdict for **every** Binding, including
//! gated ones and ones with guards still outstanding. Its grounding is not
//! `LateDataPolicy::Exclude` — that is a field the Core stamps, not a licence
//! it receives. It is this: a gated Binding has no settled conclusion at *any*
//! frontier, and the only alternative — emit it continuously as Provisional and
//! correct it later — requires an Amendment record that exists in neither
//! `proto/` nor `model/` and is open as ADR-0021's O-11. The evidence contract
//! in `crate::evidence` is what makes `seal()` a theorem rather than "trust the
//! caller".
//!
//! ## `seal()` is a universal emission licence, not an outcome pin
//!
//! It is worth saying plainly, because a reader who assumes otherwise will
//! reach two wrong conclusions in a row.
//!
//! `seal()` **already** produces a prefix-`Inconclusive` Verdict for a Scope
//! that never closes: it applies no gate check and no dedup, and emits every
//! Binding. The genuine novelty of O-03 is retiring **one** Binding mid-stream
//! *without ending the evaluation* — and if that is not said outright, a
//! reviewer will correctly argue the acceptance criterion was already met.
//!
//! What `seal()` grants is the **licence half** of retirement and nothing more.
//! `DriverSealed` is the name for that half: the driver's statement that no
//! further `Input` will arrive. It is deliberately a documentation term and
//! **not** a `crate::slice::RetirementCause` variant — leaving the variant out
//! makes minting a retirement from a seal unconstructible, where forbidding it
//! would have been a runtime refusal.
//!
//! Retirement is that licence **plus an outcome pin**, and the two therefore
//! coincide only under a **closed evidence gate**: `crate::precedence`'s
//! assessment demotes to prefix uncertainty only when the gate is closed, while
//! retirement pins unconditionally. On an open-gate fixture they disagree — and
//! retiring everything there is inadmissible anyway, because the gate opening
//! emitted first and clause (d2) refuses an emitted Binding.
//!
//! Two claims follow, and the difference between them is the whole point:
//!
//! - **False:** `seal()` and retire-every-Binding-then-`seal()` produce
//!   byte-identical Verdicts. They do not, on any fixture. Retirement adds the
//!   O-03 sentence to the envelope, and retiring mid-stream emits through
//!   [`Step::Settled`] before `seal()` emits again.
//! - **True, under its precondition:** on a fixture whose evidence gate is
//!   **closed**, the two agree on every Binding's outcome and authority. The
//!   test that asserts this asserts the precondition in its own body, with
//!   [`Evaluator::evidence_is_settled`].
//!
//! Making `seal()` actually retire every still-open Binding was considered and
//! rejected: it would pin every sealed Binding to `Inconclusive` and turn the
//! vendored-fixture suites red. The distinction is real, and it is stated here
//! rather than engineered away.
//!
//! # Determinism obligation
//!
//! ADR-0020's obligation is per **frontier**, not per fixpoint: two input
//! prefixes reaching the same causally-ready frontier with the same
//! evidence-ledger state must have settled the same set of Bindings to the same
//! Verdicts. Emission *sequence* differing across arrival orders needs no
//! disclaimer, because different arrival orders sit at different frontiers.
//! The honest gap: the evidence gate reads the ledger, which **ADR-0020's
//! frontier concept does not cover at all**. The evidence contract bounds the
//! consequences; ADR-0020 offers nothing here.
//!
//! Retirement adds one clarification of the same kind. The *bytes* a retired
//! Binding emits mid-stream do depend on arrival position — a retirement
//! recorded before a source declares emits at a different frontier than one
//! recorded after — while the **sealed** Verdict does not, because `Slice(b)`
//! is a function of the horizon join and of each Event's own observation time.
//! That is the same frontier-versus-fixpoint distinction stated above, and it
//! is what the arrival-order harness asserts on.

#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};

use sentinel_compile::ir::{AbsenceIr, AggregateIr, ExistenceIr, PrecedenceIr, TaintIr};
use sentinel_model::{
    Event, RawEventRetentionDays, SourceEvidence, SourceIdentity, SourceLivenessFreshness,
};

use crate::absence::{self, AbsenceSpecification};
use crate::aggregate::{self, AggregateSpecification};
use crate::cost::EvaluationCost;
use crate::evidence::{EvidenceGate, EvidenceLedger};
use crate::existence::{self, ExistenceSpecification};
use crate::facts::{
    ColdGuardId, EvidenceWatermarks, FactLedger, FactQuery, FactRequest, FactResponse,
    GuardConclusion,
};
use crate::frontier::{CausalIndex, CausalIndexError, Frontier};
use crate::precedence::{self, BindingConclusion};
use crate::settlement::{self, Settlement};
use crate::slice::{
    Binding, RetirementBreach, RetirementSignal, SliceIndex, SliceLifecycles, SliceRole,
};
use crate::specification::{
    PrecedenceSpecification, RetirementPolicy, SpecificationConversionError, SpecificationIdentity,
};
use crate::taint::{self, TaintSpecification};
use crate::verdict::{self, BindingVerdict};
use crate::CoreError;

/// One thing a driver can hand the Core.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Input {
    /// One adapted Event, in arrival order.
    Event(Box<Event>),
    /// Updated per-source evidence: completeness signal, gaps, tier, capabilities.
    Source(Box<SourceEvidence>),
    /// One Binding's Scope has been retired by the periphery (ADR-0021, O-03).
    ///
    /// The third thing that arrives rather than being measured. A deadline and
    /// an inactivity window are both wall-clock facts, and a Core that read a
    /// clock to evaluate them would stop being the pure evaluator ADR-0012
    /// requires — so the periphery decides, and hands over the horizon it
    /// decided on. The Core's whole contribution is comparison.
    ///
    /// Boxed beside [`Input::Event`], and for its reason:
    /// `clippy::pedantic`'s `large_enum_variant` measures the widest variant,
    /// and an unboxed signal would make every `Input` as wide as this one.
    Retire(Box<RetirementSignal>),
}

impl Input {
    /// Delivers one Event.
    #[must_use]
    pub fn event(event: Event) -> Self {
        Self::Event(Box::new(event))
    }

    /// Declares one source's latest evidence.
    #[must_use]
    pub fn source(evidence: SourceEvidence) -> Self {
        Self::Source(Box::new(evidence))
    }

    /// Declares that one Binding's Scope has been retired.
    #[must_use]
    pub fn retire(signal: RetirementSignal) -> Self {
        Self::Retire(Box::new(signal))
    }
}

/// What one step produced.
///
/// `'static + Send`: nothing here borrows the [`Evaluator`], so #16's async
/// store, #18's cold operators and #22 can each own an `Evaluator`, hand a
/// request to a fetcher and apply the answer later.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Step {
    /// Bindings whose hot monitor settled, and whose Verdict changed.
    Settled(Vec<BindingVerdict>),
    /// Nothing settled and nothing is being asked; more evidence may settle it.
    Pending,
    /// The batched cold-path queries that block authoritative settlement.
    NeedsFacts(FactRequest),
}

/// The latest observation time in one Binding's Slice, when there is one.
///
/// The read half of retirement: what the Core has **observed**, offered to a
/// driver that has to decide whether a Scope has gone quiet. It is not a
/// [`crate::slice::ScopeHorizon`] and does not become one — the periphery
/// computes that, and the arithmetic that turns a latest observation into a
/// deadline stays outside a Core that reads no clock.
///
/// Three-valued rather than `Option<ObservationTime>`, because an empty Slice
/// and a Slice whose observations sit on more than one Sentinel clock are
/// different answers with different repairs, and neither is a time.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SliceHorizon {
    /// No admitted member the Frontier can resolve.
    Empty,
    /// The latest observation time across the Binding's admitted members.
    Observed(sentinel_model::ObservationTime),
    /// Members observed on more than one Sentinel clock, so there is no latest.
    IncomparableClocks,
}

/// Every Binding's final Verdict, and what the evaluation cost.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Sealed {
    verdicts: Vec<BindingVerdict>,
    cost: EvaluationCost,
    closed_guards: BTreeMap<(Binding, ColdGuardId), GuardConclusion>,
}

impl Sealed {
    /// Returns one Verdict per Binding, sorted by Binding.
    #[must_use]
    pub fn verdicts(&self) -> &[BindingVerdict] {
        &self.verdicts
    }

    /// Returns what the whole evaluation cost in cold-path round trips.
    #[must_use]
    pub const fn cost(&self) -> &EvaluationCost {
        &self.cost
    }

    /// Returns settled refusals and failures, derived from the ledger.
    ///
    /// Empty when every settled query is [`GuardConclusion::Holds`] or no
    /// guard was declared. Additive on this `#[non_exhaustive]` type so a
    /// driver that only calls [`Evaluator::seal`] can still see a refusal.
    #[must_use]
    pub const fn closed_guards(&self) -> &BTreeMap<(Binding, ColdGuardId), GuardConclusion> {
        &self.closed_guards
    }

    /// Consumes the seal, returning its Verdicts.
    #[must_use]
    pub fn into_verdicts(self) -> Vec<BindingVerdict> {
        self.verdicts
    }
}

/// A resumable pure evaluator over one catalogued Pattern family.
///
/// Dispatch between CAT-1 and CAT-4 lives here, closed by the Catalogue, not by
/// a public enum named `Specification` and not by `From<&MonitorIr>` or
/// `TryFrom<&MonitorIr>` (`MonitorIr`
/// is `#[non_exhaustive]`; rustc 1.96 cannot make a third family a compile
/// error at this seam).
#[derive(Clone, Debug)]
pub struct Evaluator {
    specification: Monitored,
    frontier: Frontier,
    ledger: EvidenceLedger,
    slices: SliceIndex,
    facts: FactLedger,
    cost: EvaluationCost,
    index: CausalIndex,
    contradiction: Option<CausalIndexError>,
    emitted: BTreeMap<Binding, EmissionKey>,
    lifecycles: SliceLifecycles,
    /// Optional source-liveness floors for barrier clearing (O-12 / #163).
    ///
    /// When set, Verdict construction uses [`settlement::settle_within`] so a
    /// post-activation silence is named rather than collapsed into missing evidence.
    liveness: Option<SourceLivenessFreshness>,
    /// Raw-Event retention horizon stamped on every sealed Assurance Envelope.
    ///
    /// Defaults to [`RawEventRetentionDays::platform_default`]; production
    /// callers that know a Tenant horizon override via
    /// [`Self::with_raw_event_retention`]. Core never reads the store (ADR-0012).
    raw_event_retention: RawEventRetentionDays,
}

/// What one Binding's last emission is deduplicated against.
///
/// Two shapes, because a retired Binding and an open one settle differently.
/// See [`Evaluator::emission_key`] for why the retired shape is one bit and why
/// that bit is what preserves "at most two emissions per Binding".
#[derive(Clone, Debug, Eq, PartialEq)]
enum EmissionKey {
    /// Every non-retired Binding: the encoded envelope, exactly as before O-03.
    Envelope(Vec<u8>),
    /// A retired Binding: its outcome is a constant, so all that is left is
    /// whether authority has reached `Authoritative`.
    Retired {
        /// Advances `false -> true` at most once, and is never written back.
        authoritative: bool,
    },
}

/// Private, exhaustive family dispatch. Not a Core `Specification` dual: it
/// does not match `MonitorIr` and is not re-exported.
#[derive(Clone, Debug)]
enum Monitored {
    Precedence(PrecedenceSpecification),
    Taint(TaintSpecification),
    Absence(AbsenceSpecification),
    Aggregate(AggregateSpecification),
    Existence(ExistenceSpecification),
}

impl Monitored {
    fn identity(&self) -> &SpecificationIdentity {
        match self {
            Self::Precedence(specification) => specification.identity(),
            Self::Taint(specification) => specification.identity(),
            Self::Absence(specification) => specification.identity(),
            Self::Aggregate(specification) => specification.identity(),
            Self::Existence(specification) => specification.identity(),
        }
    }

    fn required_source_identities(&self) -> BTreeSet<SourceIdentity> {
        match self {
            Self::Precedence(specification) => specification.required_source_identities(),
            Self::Taint(specification) => specification.required_source_identities(),
            Self::Absence(specification) => specification.required_source_identities(),
            Self::Aggregate(specification) => specification.required_source_identities(),
            Self::Existence(specification) => specification.required_source_identities(),
        }
    }

    fn binding_of(&self, event: &Event) -> Option<String> {
        match self {
            Self::Precedence(specification) => specification.binding_of(event),
            Self::Taint(specification) => specification.binding_of(event),
            Self::Absence(specification) => specification.binding_of(event),
            Self::Aggregate(specification) => specification.binding_of(event),
            Self::Existence(specification) => specification.binding_of(event),
        }
    }

    fn cold_guards(&self) -> &BTreeSet<ColdGuardId> {
        match self {
            Self::Precedence(specification) => specification.cold_guards(),
            Self::Taint(specification) => specification.cold_guards(),
            Self::Absence(specification) => specification.cold_guards(),
            Self::Aggregate(specification) => specification.cold_guards(),
            Self::Existence(specification) => specification.cold_guards(),
        }
    }

    fn raises_obligation(&self, event: &Event) -> bool {
        match self {
            Self::Precedence(specification) => specification.trigger().matches(event),
            Self::Taint(specification) => specification.action().matches(event),
            Self::Absence(specification) => specification.until().matches(event),
            Self::Aggregate(specification) => specification.trigger().matches(event),
            Self::Existence(specification) => specification.open().matches(event),
        }
    }

    /// Returns what this Specification declares about a never-closing Scope.
    ///
    /// Read uniformly across every catalogued family, and lowered from
    /// `sentinel_compile::ir::MonitorIr::retirement` for a catalogued instance.
    /// That uniformity is the whole point: CAT-4 is excluded from retirement
    /// **because its Pattern declares `NotRequired`**, not because this enum
    /// has an arm that says so.
    const fn retirement_policy(&self) -> RetirementPolicy {
        match self {
            Self::Precedence(specification) => specification.retirement_policy(),
            Self::Taint(specification) => specification.retirement_policy(),
            Self::Absence(specification) => specification.retirement_policy(),
            Self::Aggregate(specification) => specification.retirement_policy(),
            Self::Existence(specification) => specification.retirement_policy(),
        }
    }
}

impl Evaluator {
    /// Creates an evaluator over one CAT-1 Specification.
    ///
    /// # Panics
    ///
    /// Never: the empty causal index is trivially acyclic.
    #[must_use]
    pub fn new(specification: PrecedenceSpecification) -> Self {
        Self::from_monitored(Monitored::Precedence(specification))
    }

    /// Creates an evaluator over one CAT-4 Specification.
    ///
    /// Named `taint`, not `new_taint`: candidate grammar C5 bans the token
    /// `new(` in `lower.rs`, so a lowering body that called `new_taint` would
    /// fail the guard. `Evaluator::new` stays CAT-1-only.
    ///
    /// # Panics
    ///
    /// Never: the empty causal index is trivially acyclic.
    #[must_use]
    pub fn taint(specification: TaintSpecification) -> Self {
        Self::from_monitored(Monitored::Taint(specification))
    }

    /// Creates an evaluator over one CAT-2 Specification.
    ///
    /// Named `absence`, not `new_absence`: candidate grammar C5 bans the token
    /// `new(` in `lower.rs`, so a lowering body that called `new_absence`
    /// would fail the guard. `Evaluator::new` stays CAT-1-only.
    ///
    /// # Panics
    ///
    /// Never: the empty causal index is trivially acyclic.
    #[must_use]
    pub fn absence(specification: AbsenceSpecification) -> Self {
        Self::from_monitored(Monitored::Absence(specification))
    }

    /// Creates an evaluator over one CAT-5 Specification.
    ///
    /// Named `aggregate`, not `new_aggregate`: candidate grammar C5 bans the
    /// token `new(` in `lower.rs`, so a lowering body that called
    /// `new_aggregate` would fail the guard. `Evaluator::new` stays CAT-1-only.
    ///
    /// # Panics
    ///
    /// Never: the empty causal index is trivially acyclic.
    #[must_use]
    pub fn aggregate(specification: AggregateSpecification) -> Self {
        Self::from_monitored(Monitored::Aggregate(specification))
    }

    /// Creates an evaluator over one CAT-3 Specification.
    ///
    /// Named `existence`, not `new_existence`: candidate grammar C5 bans the
    /// token `new(` in `lower.rs`.
    #[must_use]
    pub fn existence(specification: ExistenceSpecification) -> Self {
        Self::from_monitored(Monitored::Existence(specification))
    }

    fn from_monitored(specification: Monitored) -> Self {
        Self {
            specification,
            frontier: Frontier::new(),
            ledger: EvidenceLedger::new(),
            slices: SliceIndex::new(),
            facts: FactLedger::new(),
            cost: EvaluationCost::default(),
            index: CausalIndex::build(&[]).expect("the empty index is acyclic"),
            contradiction: None,
            emitted: BTreeMap::new(),
            lifecycles: SliceLifecycles::new(),
            liveness: None,
            raw_event_retention: RawEventRetentionDays::platform_default(),
        }
    }

    /// Attaches source-liveness floors used when clearing the activation barrier.
    ///
    /// Pure data: the evaluator still reads no clock. Callers in the periphery compute
    /// floors from an injected `now` before constructing this value.
    #[must_use]
    pub fn with_liveness_freshness(mut self, liveness: SourceLivenessFreshness) -> Self {
        self.liveness = Some(liveness);
        self
    }

    /// Overrides the raw-Event retention horizon stamped on sealed envelopes.
    ///
    /// Pure data: the evaluator still reads no store and no clock. Callers that
    /// know a Tenant-configured horizon (e.g. `api::evaluate_published`) supply
    /// an already-validated [`RawEventRetentionDays`].
    #[must_use]
    pub fn with_raw_event_retention(mut self, retention: RawEventRetentionDays) -> Self {
        self.raw_event_retention = retention;
        self
    }

    /// Steps the evaluator with one input.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::Delivery`] for a delivery conflict,
    /// [`CoreError::NonMonotonicEvidence`] when the input contradicts what the
    /// same source already declared, [`CoreError::Causality`] when the declared
    /// edges admit no partial order, and [`CoreError::Verdict`] when an
    /// assembled envelope violates a model invariant.
    ///
    /// Returns [`CoreError::SpecificationRequiresNoSource`] when the
    /// Specification declares no required source: that leaves the evidence gate
    /// trivially open, so stepping reaches Verdict construction and refuses
    /// there. [`CoreError::NoEvidenceCoverage`] is **not** reachable from here
    /// and is documented on [`Evaluator::seal`] — but the reason is no longer
    /// "stepping builds Verdicts only behind an open gate". Since O-03 a
    /// **retired** Binding builds Verdicts behind a closed one, so the licence
    /// carries a non-empty-Evidence-Coverage conjunct precisely to keep this
    /// true; for a Specification that does require a source, an open gate still
    /// *is* complete Evidence Coverage. See [`Evaluator::advance`].
    ///
    /// Returns [`CoreError::Retirement`] when an [`Input::Retire`] breaches the
    /// retirement admissibility contract — the Specification declares no policy
    /// or a different cause, the horizon regresses or is on another Sentinel
    /// clock, or the Binding has already emitted. Every clause is checked
    /// **before** any state is mutated, so a refused signal leaves the
    /// evaluator usable: an empty `resume()` succeeds and `seal()` still
    /// returns Verdicts.
    pub fn step(&mut self, input: Input) -> Result<Step, CoreError> {
        match input {
            Input::Event(event) => self.step_event(&event)?,
            Input::Source(evidence) => self.ledger.record_source(*evidence)?,
            Input::Retire(signal) => self.step_retirement(&signal)?,
        }
        self.advance()
    }

    /// Records one retirement signal, or refuses it.
    ///
    /// **Clause (d0), then clauses (d3) and (d2) inside
    /// [`SliceLifecycles::retire`], and every one of them before any mutation.**
    /// The ordering is deliberate on both counts. (d0) is a property of the
    /// Specification alone, so it is order-free and belongs first. (d3) has to
    /// precede (d2), because a retired Binding has already emitted by the time
    /// a second signal arrives and checking (d2) first would make idempotence
    /// unreachable.
    ///
    /// Clause (d0) is read off the Specification's own declaration, so a CAT-4
    /// Binding — or any Pattern declaring `NotRequired` — is refused because
    /// the catalogue says so.
    ///
    /// A Binding the Core has admitted no Event for is **admissible** here and
    /// produces no Verdict: `crate::precedence::assess` iterates the Slice
    /// index's Bindings, and an unseen Binding is not among them. Refusing it
    /// would be order-sensitive on `{Retire(b), e_of_b}`, which is precisely
    /// the arrival-position reasoning this contract exists to avoid.
    fn step_retirement(&mut self, signal: &RetirementSignal) -> Result<(), CoreError> {
        // (d0).
        let Some(declared) = self.specification.retirement_policy().declared_cause() else {
            return Err(CoreError::Retirement(RetirementBreach::NotDeclared));
        };
        if signal.cause() != declared {
            return Err(CoreError::Retirement(RetirementBreach::CauseNotDeclared));
        }
        // (d3) then (d2), with the mutation behind both.
        self.lifecycles
            .retire(signal, self.emitted.contains_key(signal.binding()))
            .map_err(CoreError::Retirement)?;
        Ok(())
    }

    /// Applies recorded fact responses and steps again.
    ///
    /// Passing an empty batch is legal and is how a driver asks "what do you
    /// need?" without supplying new evidence.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::FactResponseMismatch`] for a response echoing a
    /// query this evaluator never issued — the only malformed-input case. A
    /// response that simply does not settle its query degrades rather than
    /// erroring: the query stays outstanding and re-issues.
    ///
    /// Resuming steps the evaluator, so every error [`Evaluator::step`]
    /// documents is reachable here too.
    pub fn resume(&mut self, responses: Vec<FactResponse>) -> Result<Step, CoreError> {
        let required = self.specification.required_source_identities();
        for response in responses {
            if !self.facts.was_issued(response.query()) {
                return Err(CoreError::FactResponseMismatch {
                    binding: response.query().binding().clone(),
                    guard: response.query().guard().clone(),
                });
            }
            self.cost.record_response(response.query().binding());
            self.facts.apply(&response, &required);
        }
        self.advance()
    }

    /// Returns every query not yet answered by a settling response.
    #[must_use]
    pub fn outstanding(&self) -> Vec<FactQuery> {
        self.facts.outstanding()
    }

    /// Returns settled refusals and failures across every Binding.
    ///
    /// Derived from [`crate::facts::FactLedger`] after a query has settled.
    /// There is no `QueryState::Closed`: a terminal
    /// [`GuardConclusion::Refused`] is `Settled`.
    #[must_use]
    pub fn closed_guards(&self) -> BTreeMap<(Binding, ColdGuardId), GuardConclusion> {
        self.facts.closed_all()
    }

    /// Returns what the evaluation has cost so far.
    #[must_use]
    pub const fn cost(&self) -> &EvaluationCost {
        &self.cost
    }

    /// Returns the Slice index, which #17 instruments.
    #[must_use]
    pub const fn slices(&self) -> &SliceIndex {
        &self.slices
    }

    /// Returns the contradictory cycle the delivered edges contain, if any.
    ///
    /// While this is `Some`, every step returns [`Step::Pending`] and nothing
    /// can settle: a cyclic edge set admits no partial order, so it has no
    /// frontier valuation to read a Verdict off. [`Evaluator::seal`] raises it.
    #[must_use]
    pub const fn causal_contradiction(&self) -> Option<&CausalIndexError> {
        self.contradiction.as_ref()
    }

    /// Returns the accumulated evidence ledger.
    #[must_use]
    pub const fn ledger(&self) -> &EvidenceLedger {
        &self.ledger
    }

    /// Returns the CAT-1 Specification this evaluator evaluates, when it is one.
    ///
    /// `None` for a CAT-4 or CAT-2 evaluator: provenance/taint and absence are
    /// detect-only (ADR-0023 gate 3) and are **not a `decide()` input**. The
    /// enforcement seam treats `None` as identity mismatch — never Permit.
    ///
    /// `pub(crate)`, not public: the enforcement decision seam
    /// (`crate::decision`) is a sibling module, and this evaluator's fields
    /// are module-private, so the seam reads them through these accessors
    /// and nothing else.
    #[must_use]
    pub(crate) const fn specification(&self) -> Option<&PrecedenceSpecification> {
        match &self.specification {
            Monitored::Precedence(specification) => Some(specification),
            Monitored::Taint(_)
            | Monitored::Absence(_)
            | Monitored::Aggregate(_)
            | Monitored::Existence(_) => None,
        }
    }

    /// Returns the declared cold-path guards, empty for CAT-4.
    #[must_use]
    pub(crate) fn declared_cold_guards(&self) -> &BTreeSet<ColdGuardId> {
        self.specification.cold_guards()
    }

    /// Returns the admission frontier over the delivered prefix.
    ///
    /// `pub(crate)` beside [`Self::specification`]: the decision seam reads
    /// the already-delivered check and the admitted Slice membership off the
    /// frontier rather than through a second copy of delivered state.
    #[must_use]
    pub(crate) const fn frontier(&self) -> &Frontier {
        &self.frontier
    }

    /// Returns the transitive-reachability index over the delivered set.
    ///
    /// `pub(crate)` beside [`Self::specification`]: the decision seam
    /// classifies a not-yet-delivered action against this index, which is
    /// the one thing it cannot do with `CausalIndex::reaches` alone.
    #[must_use]
    pub(crate) const fn causal_index(&self) -> &CausalIndex {
        &self.index
    }

    /// Returns whether the evidence delivered so far can support a conclusion
    /// about what is **absent** from it.
    ///
    /// The same coordinate `advance` gates emission on — the evidence gate is
    /// open and Watermarks cover every delivered epoch — reported rather than
    /// acted on, plus the causal-contradiction check, because a cyclic edge set
    /// admits no partial order and therefore no frontier valuation to read
    /// anything off.
    ///
    /// # What it is for
    ///
    /// Issue #144: a Specification whose trigger selector matched no Event
    /// measured nothing, and that is worth reporting — but only once no further
    /// Event can arrive. Before that point the trigger has matched nothing
    /// *yet*, and saying so would be a finding about the clock rather than about
    /// the Specification: the mirror image of the silent pass #144 names.
    ///
    /// `sentinel_compile::catalogue::SettledEvidence` is the parameter type that
    /// carries that assertion into the census. It cannot recompute this
    /// condition — `compile` never sees a Frontier — so this is the accessor a
    /// driver asks before minting one. The assertion stays the caller's, which
    /// is deliberate: the Core does not know how many Evaluators a validation
    /// corpus spans.
    #[must_use]
    pub fn evidence_is_settled(&self) -> bool {
        if self.contradiction.is_some() {
            return false;
        }
        let gate = EvidenceGate::evaluate(
            self.specification.required_source_identities(),
            &self.ledger,
            &self.frontier,
        );
        gate.is_open() && gate.watermarks_cover_epochs()
    }

    /// Returns whether one Binding has already emitted a Verdict.
    ///
    /// The published guard on clause (d2). [`Evaluator::evidence_is_settled`] is
    /// a *global* proxy and would answer "not settled" for a Binding that has
    /// nonetheless emitted, so a driver deciding whether a retirement signal is
    /// still admissible needs this instead. With it published, (d2) is a
    /// contract with a guard rather than a trap.
    #[must_use]
    pub fn is_emitted(&self, binding: &Binding) -> bool {
        self.emitted.contains_key(binding)
    }

    /// Returns the latest observation time in one Binding's Slice.
    ///
    /// **Not a retirement decision, and not [`crate::slice::ScopeHorizon`].**
    /// This is a read of what the Core has observed; a `ScopeHorizon` is what
    /// the periphery decided. The two meet only when a driver uses this value
    /// to compute one — an inactivity policy is `slice_horizon(b) + window`,
    /// and that addition, with its overflow surface, stays deliberately outside
    /// the Core.
    ///
    /// Three-valued, because "no observation" and "observations the Core cannot
    /// order" are different answers and neither is a time. Collapsing either
    /// into a value would hand a driver a horizon the evidence does not
    /// support.
    #[must_use]
    pub fn slice_horizon(&self, binding: &Binding) -> SliceHorizon {
        let Some(members) = self.slices.members(binding) else {
            return SliceHorizon::Empty;
        };
        let mut latest: Option<sentinel_model::ObservationTime> = None;
        for id in members {
            let Some(event) = self.frontier.event(id) else {
                continue;
            };
            let observed = event.observation_time();
            match &latest {
                None => latest = Some(observed),
                Some(current) => {
                    if current.sentinel_clock() != observed.sentinel_clock() {
                        return SliceHorizon::IncomparableClocks;
                    }
                    if observed.sentinel_nanoseconds() > current.sentinel_nanoseconds() {
                        latest = Some(observed);
                    }
                }
            }
        }
        latest.map_or(SliceHorizon::Empty, SliceHorizon::Observed)
    }

    /// Declares that no further input will arrive, and emits every Verdict.
    ///
    /// # A universal emission licence, not a universal retirement
    ///
    /// This is the driver's `DriverSealed` statement: no further [`Input`] is
    /// coming. It emits every Binding, applying no gate check and no dedup, so
    /// a Scope that never closed does get a prefix-`Inconclusive` Verdict here —
    /// **without** anything having been retired.
    ///
    /// It is **not** an outcome pin. Sealing lets a Binding whose gate is open
    /// settle whatever its evidence settles, up to and including a
    /// watermark-anchored `Violate`; retiring a Binding fixes its outcome to
    /// `Inconclusive` whatever the gate says. The two coincide only under a
    /// **closed** evidence gate, where the assessment demotes to prefix
    /// uncertainty for its own reasons.
    ///
    /// So "`seal()` is universal retirement" is false, and the two testable
    /// claims are: byte-identity against retire-all-then-seal does **not** hold
    /// on any fixture, and equal outcomes and authorities per Binding **does**,
    /// under the closed-gate precondition. `DriverSealed` names the licence half
    /// and is deliberately not a `crate::slice::RetirementCause` variant.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::Causality`] when the delivered edges admit no
    /// partial order, and [`CoreError::Verdict`] when an assembled envelope
    /// violates a model invariant.
    ///
    /// Returns [`CoreError::NoEvidenceCoverage`] when at least one Binding
    /// exists and no required source supplied *both* delivered Events and a
    /// declared `SourceEvidence`. Sealing does not require an open gate, which
    /// is what makes it the only entry point that reaches an Assurance Envelope
    /// with no Evidence Coverage to build one from. Returns
    /// [`CoreError::SpecificationRequiresNoSource`] when the Specification
    /// declares no required source at all.
    pub fn seal(self) -> Result<Sealed, CoreError> {
        if let Some(error) = self.contradiction {
            return Err(CoreError::Causality(error));
        }
        let gate = EvidenceGate::evaluate(
            self.specification.required_source_identities(),
            &self.ledger,
            &self.frontier,
        );
        let mut verdicts = self.build_verdicts(&gate)?;
        verdicts.sort_by(|left, right| left.key().cmp(right.key()));
        Ok(Sealed {
            verdicts,
            cost: self.cost,
            closed_guards: self.facts.closed_all(),
        })
    }

    fn step_event(&mut self, event: &Event) -> Result<(), CoreError> {
        // Rule (b′): `Frontier::step` classifies delivery conflicts **first**,
        // so a conflicting delivery is always reported as a conflict and never
        // as a contract breach. This ordering is a permanent rule, not a
        // consequence of the driver feeding Events before Sources.
        let admission = self.frontier.step(event.clone())?;
        self.ledger
            .record_delivery(event, admission.is_new_delivery())?;

        for id in admission.newly_admitted() {
            let Some(admitted) = self.frontier.event(id) else {
                continue;
            };
            let Some(binding) = self.specification.binding_of(admitted) else {
                continue;
            };
            let role = if self.specification.raises_obligation(admitted) {
                SliceRole::ObligationRaising
            } else {
                SliceRole::Member
            };
            self.slices.admit(&Binding::new(binding), id.clone(), role);
        }

        if admission.is_new_delivery() {
            // Eager, per step. Deferring the *rebuild* until a Verdict is
            // demanded would let a driver emit settled Verdicts and only
            // discover at `seal()` that the declared edges admit no partial
            // order at all, with nothing to retract them with.
            match CausalIndex::build(&self.frontier.delivered()) {
                Ok(index) => {
                    self.index = index;
                    self.contradiction = None;
                }
                // Rule (b′), generalised: an error that classifies **this
                // delivery** outranks one derived from the accumulated
                // delivered set. A contradictory edge set is detected the
                // moment it appears and blocks every emission from that moment
                // (see `advance`), so nothing unsound can be settled — but it
                // is *raised* when the driver stops supplying input, so a later
                // delivery that conflicts on identity or slot is still reported
                // as the conflict it is rather than masked by a cycle that
                // conflict itself created. `causal_contradiction` exposes it
                // immediately to a driver that wants to stop early.
                Err(error) => self.contradiction = Some(error),
            }
        }
        Ok(())
    }

    /// Emits whatever changed, then asks for whatever is still outstanding.
    ///
    /// Emissions drain first so a hot settlement is never swallowed by the
    /// request it creates; the queries it declared are issued on the following
    /// call, which may be `resume(Vec::new())`.
    ///
    /// # The retired emission licence
    ///
    /// Step 4 of this module's theorem, in three edits rather than one widened
    /// condition, because the branch below guards two different things:
    ///
    /// 1. [`Evaluator::declare_queries`] stays **gate-open only**. The cold
    ///    floor is snapshotted into a `crate::facts::FactQuery` once and never
    ///    refreshed, so declaring behind a closed gate installs a permanently
    ///    lower floor and lets a cold answer settle a conjunct it never saw.
    /// 2. The emission licence gains the retired disjunct, with a **non-empty
    ///    Evidence Coverage** conjunct — without it,
    ///    [`Evaluator::refuse_unconstructible_envelope`] would return
    ///    [`CoreError::NoEvidenceCoverage`] from `step()` with the lifecycle
    ///    mutation already committed, and wedge the evaluator.
    /// 3. The licence is applied **per Binding**, inside the loop where the
    ///    Binding is bound. Widening the branch alone would emit every open
    ///    Binding behind a closed gate and destroy Steps 1–3's premise.
    ///
    /// `any_retired()` short-circuits the whole thing: with nothing retired,
    /// Evidence Coverage is never computed, `retired_licence` is `false`, and
    /// this reduces to exactly the branch that was here before O-03. That is
    /// what the untouched suites prove.
    ///
    /// The accepted cost: under the retired licence with the gate closed,
    /// `build_verdicts` still assembles envelopes for open Bindings that the
    /// `continue` then discards. Wasted work, not a correctness problem.
    fn advance(&mut self) -> Result<Step, CoreError> {
        if self.contradiction.is_some() {
            // No partial order exists, so no frontier valuation does either.
            return Ok(Step::Pending);
        }
        let gate = EvidenceGate::evaluate(
            self.specification.required_source_identities(),
            &self.ledger,
            &self.frontier,
        );
        let mut emissions = Vec::new();
        let gate_ok = gate.is_open() && gate.watermarks_cover_epochs();
        // One statement: `Coverage` borrows the ledger and `declare_queries`
        // takes `&mut self`, so holding it across the call would be `E0502`.
        let retired_licence = self.lifecycles.any_retired()
            && !self
                .ledger
                .coverage(self.specification.required_source_identities())
                .is_empty();
        if gate_ok {
            self.declare_queries();
        }
        if gate_ok || retired_licence {
            for verdict in self.build_verdicts(&gate)? {
                let retired = self.lifecycles.is_retired(verdict.key());
                if !gate_ok && !retired {
                    continue;
                }
                let key = self.emission_key(&verdict, retired);
                if self.emitted.get(verdict.key()) == Some(&key) {
                    continue;
                }
                self.emitted.insert(verdict.key().clone(), key);
                emissions.push(verdict);
            }
        }

        if !emissions.is_empty() {
            emissions.sort_by(|left, right| left.key().cmp(right.key()));
            return Ok(Step::Settled(emissions));
        }
        if let Some((request, bindings)) = self.facts.issue() {
            self.cost.record_round_trip();
            for binding in &bindings {
                self.cost.record_issue(binding, request.round_trip());
            }
            return Ok(Step::NeedsFacts(request));
        }
        Ok(Step::Pending)
    }

    /// Returns the dedup key one Verdict is emitted under.
    ///
    /// Step 5 of this module's theorem. An open Binding keys on the encoded
    /// envelope, exactly as before O-03. A retired Binding keys on **one bit**,
    /// because its outcome is a constant under Step 4 and the only decisive
    /// content left is whether authority has reached `Authoritative`.
    ///
    /// The bit is **sticky by construction**: it is joined with whatever was
    /// last emitted, so it advances `false -> true` at most once and is never
    /// written back. That is what keeps "at most two emissions per Binding"
    /// true for a Binding emitting behind a closed gate, where
    /// `(authority, cold_path)` can otherwise take three values in sequence.
    /// Declining to re-emit an `Authoritative -> Provisional` regression is
    /// independently correct: `sentinel-store` refuses exactly that transition.
    ///
    /// The two key shapes cannot mix for one Binding. Clause (d2) refuses to
    /// retire a Binding that has already emitted, so an `Envelope` key never
    /// becomes a `Retired` one.
    fn emission_key(&self, verdict: &BindingVerdict, retired: bool) -> EmissionKey {
        if !retired {
            return EmissionKey::Envelope(verdict.verdict().encode_to_vec());
        }
        let already_authoritative = matches!(
            self.emitted.get(verdict.key()),
            Some(EmissionKey::Retired {
                authoritative: true
            })
        );
        EmissionKey::Retired {
            authoritative: already_authoritative
                || verdict.verdict().authority() == sentinel_model::VerdictAuthority::Authoritative,
        }
    }

    /// Declares one query per `(Binding, declared guard)`, at the hot floor.
    ///
    /// The floor is the hot-path settlement point: an answer computed behind it
    /// has not seen what the hot path saw, and cannot settle the conjunction.
    ///
    /// **Gate-open only, deliberately.** `crate::facts::FactLedger::declare` is
    /// idempotent by `(binding, guard)`, so the floor below is snapshotted once
    /// and never refreshed; declaring it behind a closed gate would install a
    /// permanently lower floor. A retired guard-declaring Specification
    /// therefore never has a query declared while gated, stays `PENDING` and
    /// never upgrades — a disclosed limitation, and #18's to close.
    fn declare_queries(&mut self) {
        if self.specification.cold_guards().is_empty() {
            return;
        }
        let floor = self.evidence_watermarks().into_floor();
        let bindings: Vec<Binding> = self.slices.bindings().cloned().collect();
        for binding in bindings {
            if self
                .slices
                .obligation_raising(&binding)
                .is_none_or(std::collections::BTreeSet::is_empty)
            {
                continue;
            }
            for guard in self.specification.cold_guards() {
                self.facts.declare(FactQuery::new(
                    guard.clone(),
                    binding.clone(),
                    floor.clone(),
                ));
            }
        }
    }

    /// Returns the Core's own per-source Watermark over the required sources.
    ///
    /// One coordinate, read for two opposite purposes, and its being *one*
    /// coordinate is the whole of #108's fix. [`Evaluator::declare_queries`]
    /// reads it as the **floor** an answer must reach: a cold path computed
    /// behind the hot path has not seen what the hot path saw.
    /// [`Evaluator::build_verdicts`] reads it as the **ceiling** a recorded
    /// cold settlement may not exceed, because `sentinel_model`'s
    /// `validate_settlement_reached` demands `evidence[s] >= cold[s]` against
    /// exactly the Watermarks [`crate::verdict`] writes into
    /// `assurance.watermarks` — which are these, derived from the same
    /// completeness signals. Deriving the ceiling anywhere else would be a
    /// second source of truth for a value the model compares for equality's
    /// sake, and the two copies would drift.
    ///
    /// The two readings are separated in time, and that separation is real
    /// rather than incidental: [`crate::facts::FactLedger::declare`] is
    /// idempotent by `(binding, guard)`, so the floor is snapshotted into a
    /// [`crate::facts::FactQuery`] once and never refreshed, while the ceiling
    /// is whatever the ledger holds when a Verdict is assembled. Evidence is
    /// monotone (`MonotonicityBreach::RegressedEpoch`), so ceiling `>=` floor
    /// always — but *strictly* greater is reachable, and that is why
    /// [`crate::settlement`]'s clamp needs an arm for an answer sitting in an
    /// epoch the source has since left.
    ///
    /// A required source whose completeness boundary is absent is **omitted**
    /// rather than recorded at zero, which makes this map deliberately
    /// stricter than the envelope it is compared against: [`crate::verdict`]
    /// writes `complete_through: 0` for such a source, so the model would
    /// accept a cold coordinate there that the clamp instead degrades on. The
    /// Core has observed no boundary for that source, and declining to record
    /// one is the conservative direction.
    fn evidence_watermarks(&self) -> EvidenceWatermarks {
        let mut watermarks = BTreeMap::new();
        for source in self.specification.required_source_identities() {
            let Some(record) = self.ledger.source(&source) else {
                continue;
            };
            let completeness = record.completeness();
            let Some(complete_through) = completeness.complete_through() else {
                continue;
            };
            watermarks.insert(source, (completeness.epoch(), complete_through));
        }
        EvidenceWatermarks::new(watermarks)
    }

    fn build_verdicts(&self, gate: &EvidenceGate) -> Result<Vec<BindingVerdict>, CoreError> {
        match &self.specification {
            Monitored::Precedence(specification) => {
                self.build_precedence_verdicts(specification, gate)
            }
            Monitored::Taint(specification) => self.build_taint_verdicts(specification, gate),
            Monitored::Absence(specification) => self.build_absence_verdicts(specification, gate),
            Monitored::Aggregate(specification) => {
                self.build_aggregate_verdicts(specification, gate)
            }
            Monitored::Existence(specification) => {
                self.build_existence_verdicts(specification, gate)
            }
        }
    }

    fn build_precedence_verdicts(
        &self,
        specification: &PrecedenceSpecification,
        gate: &EvidenceGate,
    ) -> Result<Vec<BindingVerdict>, CoreError> {
        let required = specification.required_source_identities();
        let coverage = self.ledger.coverage(required.clone());
        let conclusions = precedence::assess(
            specification,
            &self.index,
            &self.frontier,
            &self.slices,
            &self.lifecycles,
            gate,
            &self.ledger,
        );
        self.refuse_unconstructible_envelope(conclusions.is_empty(), &coverage)?;
        let evidence_watermarks = self.evidence_watermarks();
        let mut verdicts = Vec::with_capacity(conclusions.len());
        for conclusion in &conclusions {
            let binding = conclusion.key();
            let every_query_settled =
                self.facts.has_queries(binding) && self.facts.all_settled(binding);
            let settled_watermarks = self.facts.settled_watermarks(binding);
            let settlement: Settlement = match &self.liveness {
                Some(liveness) => settlement::settle_within(
                    specification,
                    &coverage,
                    every_query_settled,
                    &settled_watermarks,
                    &evidence_watermarks,
                    liveness,
                ),
                None => settlement::settle(
                    specification,
                    &coverage,
                    every_query_settled,
                    &settled_watermarks,
                    &evidence_watermarks,
                ),
            };
            let cost = self.cost.binding(binding).cloned().unwrap_or_default();
            let any_refused = self.facts.any_refused(binding);
            // The two build paths are separate functions, not one function with
            // a flag. `verdict::build_retired` takes a `RetiredAssessment`,
            // which carries no outcome, so `Violate` and `Satisfy` are
            // unconstructible on the retired path rather than cleared from it.
            verdicts.push(match conclusion {
                BindingConclusion::Open(assessment) => verdict::build(
                    specification,
                    &self.ledger,
                    &self.frontier,
                    gate,
                    assessment,
                    &settlement,
                    cost,
                    any_refused,
                    self.raw_event_retention,
                )?,
                BindingConclusion::Retired(assessment, retirement) => verdict::build_retired(
                    specification,
                    &self.ledger,
                    &self.frontier,
                    gate,
                    assessment,
                    retirement,
                    &settlement,
                    cost,
                    any_refused,
                    self.raw_event_retention,
                )?,
            });
        }
        Ok(verdicts)
    }

    fn build_taint_verdicts(
        &self,
        specification: &TaintSpecification,
        gate: &EvidenceGate,
    ) -> Result<Vec<BindingVerdict>, CoreError> {
        let required = specification.required_source_identities();
        let coverage = self.ledger.coverage(required.clone());
        let assessments = taint::assess(
            specification,
            &self.index,
            &self.frontier,
            &self.slices,
            gate,
            &self.ledger,
        );
        self.refuse_unconstructible_envelope(assessments.is_empty(), &coverage)?;
        let evidence_watermarks = self.evidence_watermarks();
        let mut verdicts = Vec::with_capacity(assessments.len());
        for assessment in &assessments {
            let binding = assessment.key();
            let every_query_settled =
                self.facts.has_queries(binding) && self.facts.all_settled(binding);
            let settled_watermarks = self.facts.settled_watermarks(binding);
            let settlement: Settlement = match &self.liveness {
                Some(liveness) => settlement::settle_taint_within(
                    specification,
                    &coverage,
                    every_query_settled,
                    &settled_watermarks,
                    &evidence_watermarks,
                    liveness,
                ),
                None => settlement::settle_taint(
                    specification,
                    &coverage,
                    every_query_settled,
                    &settled_watermarks,
                    &evidence_watermarks,
                ),
            };
            verdicts.push(verdict::build_taint(
                specification,
                &self.ledger,
                &self.frontier,
                gate,
                assessment,
                &settlement,
                self.cost.binding(binding).cloned().unwrap_or_default(),
                self.facts.any_refused(binding),
                self.raw_event_retention,
            )?);
        }
        Ok(verdicts)
    }

    fn build_absence_verdicts(
        &self,
        specification: &AbsenceSpecification,
        gate: &EvidenceGate,
    ) -> Result<Vec<BindingVerdict>, CoreError> {
        let required = specification.required_source_identities();
        let coverage = self.ledger.coverage(required.clone());
        let assessments = absence::assess(
            specification,
            &self.index,
            &self.frontier,
            &self.slices,
            gate,
            &self.ledger,
        );
        self.refuse_unconstructible_envelope(assessments.is_empty(), &coverage)?;
        let evidence_watermarks = self.evidence_watermarks();
        let mut verdicts = Vec::with_capacity(assessments.len());
        for assessment in &assessments {
            let binding = assessment.key();
            let every_query_settled =
                self.facts.has_queries(binding) && self.facts.all_settled(binding);
            let settled_watermarks = self.facts.settled_watermarks(binding);
            let settlement: Settlement = match &self.liveness {
                Some(liveness) => settlement::settle_absence_within(
                    specification,
                    &coverage,
                    every_query_settled,
                    &settled_watermarks,
                    &evidence_watermarks,
                    liveness,
                ),
                None => settlement::settle_absence(
                    specification,
                    &coverage,
                    every_query_settled,
                    &settled_watermarks,
                    &evidence_watermarks,
                ),
            };
            verdicts.push(verdict::build_absence(
                specification,
                &self.ledger,
                &self.frontier,
                gate,
                assessment,
                &settlement,
                self.cost.binding(binding).cloned().unwrap_or_default(),
                self.facts.any_refused(binding),
                self.raw_event_retention,
            )?);
        }
        Ok(verdicts)
    }

    fn build_aggregate_verdicts(
        &self,
        specification: &AggregateSpecification,
        gate: &EvidenceGate,
    ) -> Result<Vec<BindingVerdict>, CoreError> {
        let required = specification.required_source_identities();
        let coverage = self.ledger.coverage(required.clone());
        let assessments = aggregate::assess(
            specification,
            &self.frontier,
            &self.slices,
            gate,
            &self.ledger,
            &self.facts,
        );
        self.refuse_unconstructible_envelope(assessments.is_empty(), &coverage)?;
        let evidence_watermarks = self.evidence_watermarks();
        let mut verdicts = Vec::with_capacity(assessments.len());
        for assessment in &assessments {
            let binding = assessment.key();
            let every_query_settled =
                self.facts.has_queries(binding) && self.facts.all_settled(binding);
            let settled_watermarks = self.facts.settled_watermarks(binding);
            let settlement: Settlement = match &self.liveness {
                Some(liveness) => settlement::settle_aggregate_within(
                    specification,
                    &coverage,
                    every_query_settled,
                    &settled_watermarks,
                    &evidence_watermarks,
                    liveness,
                ),
                None => settlement::settle_aggregate(
                    specification,
                    &coverage,
                    every_query_settled,
                    &settled_watermarks,
                    &evidence_watermarks,
                ),
            };
            verdicts.push(verdict::build_aggregate(
                specification,
                &self.ledger,
                &self.frontier,
                gate,
                assessment,
                &settlement,
                self.cost.binding(binding).cloned().unwrap_or_default(),
                self.facts.any_refused(binding),
                self.raw_event_retention,
            )?);
        }
        Ok(verdicts)
    }

    fn build_existence_verdicts(
        &self,
        specification: &ExistenceSpecification,
        gate: &EvidenceGate,
    ) -> Result<Vec<BindingVerdict>, CoreError> {
        let required = specification.required_source_identities();
        let coverage = self.ledger.coverage(required.clone());
        let conclusions = existence::assess(
            specification,
            &self.index,
            &self.frontier,
            &self.slices,
            &self.lifecycles,
            gate,
            &self.ledger,
        );
        self.refuse_unconstructible_envelope(conclusions.is_empty(), &coverage)?;
        let evidence_watermarks = self.evidence_watermarks();
        let mut verdicts = Vec::with_capacity(conclusions.len());
        for conclusion in &conclusions {
            let binding = conclusion.key();
            let every_query_settled =
                self.facts.has_queries(binding) && self.facts.all_settled(binding);
            let settled_watermarks = self.facts.settled_watermarks(binding);
            let settlement: Settlement = match &self.liveness {
                Some(liveness) => settlement::settle_existence_within(
                    specification,
                    &coverage,
                    every_query_settled,
                    &settled_watermarks,
                    &evidence_watermarks,
                    liveness,
                ),
                None => settlement::settle_existence(
                    specification,
                    &coverage,
                    every_query_settled,
                    &settled_watermarks,
                    &evidence_watermarks,
                ),
            };
            let cost = self.cost.binding(binding).cloned().unwrap_or_default();
            let any_refused = self.facts.any_refused(binding);
            verdicts.push(match conclusion {
                existence::ExistenceConclusion::Open(assessment) => verdict::build_existence(
                    specification,
                    &self.ledger,
                    &self.frontier,
                    gate,
                    assessment,
                    &settlement,
                    cost,
                    any_refused,
                    self.raw_event_retention,
                )?,
                existence::ExistenceConclusion::Retired(assessment, retirement) => {
                    verdict::build_existence_retired(
                        specification,
                        &self.ledger,
                        &self.frontier,
                        gate,
                        assessment,
                        retirement,
                        &settlement,
                        cost,
                        any_refused,
                        self.raw_event_retention,
                    )?
                }
            });
        }
        Ok(verdicts)
    }

    /// Shared refusals: an empty required-source set, or no Evidence Coverage
    /// at all, cannot produce a Verdict. Checked after assessment so a Binding
    /// that raised no obligation still produces none rather than an error.
    fn refuse_unconstructible_envelope(
        &self,
        assessments_empty: bool,
        coverage: &crate::evidence::Coverage<'_>,
    ) -> Result<(), CoreError> {
        if assessments_empty {
            return Ok(());
        }
        // Checked before Evidence Coverage, because an empty required-source
        // set makes `Coverage::is_empty` vacuously true and would otherwise be
        // reported as an evidence failure naming no sources at all.
        if coverage.required().is_empty() {
            return Err(CoreError::SpecificationRequiresNoSource {
                specification: self.specification.identity().clone(),
            });
        }
        // Reachable only from `Evaluator::seal`, and since O-03 that needs two
        // reasons rather than one. Behind an *open* gate, a Specification with
        // at least one required source has complete coverage by construction.
        // Behind a *closed* one, `Evaluator::advance` builds Verdicts only
        // under the retired licence, whose non-empty-Evidence-Coverage conjunct
        // exists precisely so this arm cannot fire from `step()` — where the
        // lifecycle mutation has already committed and the error would wedge
        // the evaluator. Widening that licence means re-checking this.
        if coverage.is_empty() {
            return Err(CoreError::NoEvidenceCoverage {
                required: coverage.required().clone(),
                undeclared: coverage.undeclared().clone(),
            });
        }
        Ok(())
    }
}

/// Lowers catalogued CAT-1 IR into an Evaluator.
///
/// Delegates to `PrecedenceSpecification::try_from` so a blank Binding
/// attribute or identity is `Err`, not an Evaluator whose `evaluate`
/// would return `Ok(vec![])`.
///
/// # Errors
///
/// Returns [`SpecificationConversionError`] when identity or the Binding
/// attribute is blank.
impl TryFrom<&PrecedenceIr> for Evaluator {
    type Error = SpecificationConversionError;

    fn try_from(lowered: &PrecedenceIr) -> Result<Self, Self::Error> {
        Ok(Self::new(PrecedenceSpecification::try_from(lowered)?))
    }
}

/// Lowers catalogued CAT-4 IR into an Evaluator.
///
/// Delegates to `TaintSpecification::try_from` so poison stays `Err`.
/// Barrier-free, because `TaintIr` has no activation-barrier field.
///
/// # Errors
///
/// Returns [`SpecificationConversionError`] when identity or the Binding
/// attribute is blank.
impl TryFrom<&TaintIr> for Evaluator {
    type Error = SpecificationConversionError;

    fn try_from(lowered: &TaintIr) -> Result<Self, Self::Error> {
        Ok(Self::taint(TaintSpecification::try_from(lowered)?))
    }
}

/// Lowers catalogued CAT-2 IR into an Evaluator.
///
/// Delegates to `AbsenceSpecification::try_from` so poison stays `Err`.
/// Barrier-free, because `AbsenceIr` has no activation-barrier field.
///
/// # Errors
///
/// Returns [`SpecificationConversionError`] when identity or the Binding
/// attribute is blank.
impl TryFrom<&AbsenceIr> for Evaluator {
    type Error = SpecificationConversionError;

    fn try_from(lowered: &AbsenceIr) -> Result<Self, Self::Error> {
        Ok(Self::absence(AbsenceSpecification::try_from(lowered)?))
    }
}

/// Lowers catalogued CAT-5 IR into an Evaluator.
///
/// Delegates to `AggregateSpecification::try_from` so poison stays `Err`.
/// Declares the derived count-within cold guard; barrier-free because
/// `AggregateIr` has no activation-barrier field.
///
/// # Errors
///
/// Returns [`SpecificationConversionError`] when identity or the Binding
/// attribute is blank.
impl TryFrom<&AggregateIr> for Evaluator {
    type Error = SpecificationConversionError;

    fn try_from(lowered: &AggregateIr) -> Result<Self, Self::Error> {
        Ok(Self::aggregate(AggregateSpecification::try_from(lowered)?))
    }
}

/// Lowers catalogued CAT-3 IR into an Evaluator.
///
/// Delegates to `ExistenceSpecification::try_from` so poison stays `Err`.
///
/// # Errors
///
/// Returns [`SpecificationConversionError`] when identity or the Binding
/// attribute is blank.
impl TryFrom<&ExistenceIr> for Evaluator {
    type Error = SpecificationConversionError;

    fn try_from(lowered: &ExistenceIr) -> Result<Self, Self::Error> {
        Ok(Self::existence(ExistenceSpecification::try_from(lowered)?))
    }
}
