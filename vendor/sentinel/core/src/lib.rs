//! Verification Core: a resumable pure evaluator over a partial order.
//!
//! Stepping the [`evaluator::Evaluator`] with an [`evaluator::Input`] returns
//! `Settled`, `Pending`, or `NeedsFacts(request)`; the periphery performs the
//! query and calls [`evaluator::Evaluator::resume`]. The Core performs no I/O,
//! holds no async runtime and reads no clock (ADR-0012) — and that is a
//! **mechanical gate**, not a claim: `.github/scripts/check-core-purity.sh`
//! runs in `just lint` and fails the build on a banned import, an `async fn`,
//! an `.await`, or a normal-edge dependency outside `sentinel-model` and
//! `sentinel-compile`.
//!
//! Everything time shaped arrives as an input. `Input::Source` carries
//! [`sentinel_model::SourceEvidence`]; `Input::Event` carries one adapted
//! [`sentinel_model::Event`]. Neither can be synthesized from inside this
//! crate, which is what makes "recorded fixtures, no database, no clock"
//! literal rather than aspirational.
//!
//! # What is general, and what is still not
//!
//! **General now (issue #12):** the resumable stepping protocol, the fact
//! round trip and its Watermark effect, batching and dedup of fact requests,
//! per-Verdict fact-request counting, the evidence monotonicity contract that
//! makes `seal()` sound, the Slice index, and tier read from the evidence
//! rather than from the Specification.
//!
//! **Still not general, with the issue that owns each:** no store and no store
//! trait (#16); no cold-path query language or relational operators (those
//! live in `sentinel-coldpath`); no outcome composition and no Amendments
//! (#20, and ADR-0021's O-11 — they stay one problem); #18 closed the
//! terminal-`Refused` → Authoritative `Inconclusive(epistemic)` envelope;
//! #110 / #210 closed Slice retirement's evaluator half (ADR-0021's O-03) —
//! [`evaluator::Input::Retire`], the admissibility contract, the amended
//! emission theorem and the outcome pin — and #210 lands CAT-3
//! `existence.before-scope-close` (`ScopeKind::Between` +
//! `RetirementPolicy::Deadline`) so the mechanism is reachable from catalogue
//! data (detect-only); the O-03 signal itself remains unstructured free text
//! (`INCONCLUSIVE_REASON_RETIRED` = 6 deferred); no
//! coenable GC, enable-set pruning, vector clocks, per-Slice reachability,
//! memory bounds, per-Slice monitor state, incremental `CausalIndex` or
//! per-Binding G4 scoping (#17); the activation barrier is honoured
//! ([`settlement::BarrierState`]) but nothing here publishes an Event Filter
//! that declares one, so no Candidate lifecycle and no Filter-epoch
//! publication or acknowledgement (#19); Evaluator dispatch covers every
//! catalogued family (CAT-1–CAT-5) behind a private
//! `Monitored` enum, not a public Core `Specification` dual (#197); no Pattern
//! catalogue in this crate -- #14a put it in `sentinel-compile`, which lowers
//! into the types here (#14/#15); no retry
//! budget or staleness bound on a re-issued query (#18, ADR-0021's O-21a); no
//! proto or wire change of any kind. The enforcement decision seam
//! ([`decision::decide`]) exists, but nothing serves it: no transport, no
//! `SentinelDecisionService` server and no idempotency or replay (#26/#166,
//! with #166 persisting the `AvailabilityContract` the seam demands), and no
//! decision-latency metric or fail-mode-trigger-rate counter (#25/#36,
//! ADR-0014's enforcement metrics).
//!
//! ADR-0020's open question O-20a is **resolved by ADR-0025**: Patterns compile
//! to scoped cut-lattice evaluators (PaBTL hot, PaCTL-class cold). LTrL₃
//! remains invariance grounding, not the compilation target. #12 generalises
//! the Core in mechanism and not in catalogue breadth, so it adds no second
//! property and therefore no new data point.
//!
//! # Admitted limitations
//!
//! These are named for #16/#18/#19 to inherit knowingly rather than discover.
//! None is fixed here; fixing any of them would be scope creep into an issue
//! that owns it.
//!
//! 1. **(Inherited, #10.)** Partial coverage is labelled `Provisional` /
//!    `cold = PENDING` for a Specification that requires no cold-path guard at
//!    all. ADR-0021 defines provisional as "settled by the hot path but
//!    awaiting cold-path confirmation", which is not what is happening. The
//!    model has no way to say "Inconclusive, hot settled, no cold guard
//!    required, a required source is missing". #12 preserves this behaviour
//!    verbatim rather than silently replacing it.
//! 2. **(New.)** A `Provisional` Verdict is **structurally incapable of
//!    recording that a guard answered or refused**: the model requires
//!    `cold_settlement_watermarks` to be exactly empty for a `Pending` cold
//!    path. A partly-answered guard set is indistinguishable from an unanswered
//!    one in the envelope. The vehicle is [`evaluator::Evaluator::closed_guards`],
//!    not the envelope; the envelope half of a *settled* refusal is #18's
//!    (`epistemic_from_refused`). Composition of a `Fails` stays #20.
//! 3. **(New.)** The model accepts `(Provisional, any outcome, no basis)`. Once
//!    the Core emits Provisional Verdicts with outstanding guards, a
//!    **Provisional `Violate` carrying no witness, no proof rule and no anchor
//!    evidence** becomes reachable and passes validation. #18 did not make
//!    that path more reachable: a `Fails` answer does not mint a `Violate`.
//! 4. **(New.)** The evidence gate reads the evidence ledger, which **ADR-0020's
//!    frontier concept does not cover**. The monotonicity contract bounds the
//!    consequences; ADR-0020 offers nothing here.
//! 5. **(New.)** The model requires decisive witnesses to be settled under
//!    `cold_settlement_watermarks` once those are non-empty, so an honest
//!    `cold = SETTLED` makes a hot witness *harder* to justify than
//!    `NOT_REQUIRED` would. The model rewards under-reporting cold settlement.
//!    #18 inherits this.
//! 6. **(New.)** `authority = Authoritative iff cold_path != PENDING and
//!    activation_barrier != PENDING` is sound **only because `hot_path` is
//!    unconditionally `Settled`**. A guard-free complete-coverage warming
//!    Verdict is `Provisional` + `cold = NOT_REQUIRED` + `activation_barrier =
//!    PENDING`, so the pre-#164 biconditional (`Authoritative iff cold !=
//!    PENDING`) is false. #18 introducing real hot-path settlement states must
//!    revisit that rule; the model will reject the mistake, but loudly and late.
//! 7. **(New.)** Contract clause (c) makes settlement conditional on the Core
//!    receiving **every** Event up to a required source's declared boundary. A
//!    source that declares a boundary it never fully delivers leaves every
//!    dependent Binding permanently unsettled, with no timeout. That is the
//!    honest behaviour absent a clock, and an interior hole stalls as surely as
//!    a missing tail. Same shape as ADR-0021's O-21a — #18. Clause (c) is also
//!    **scoped to required sources**, so a non-required source's receipt hole
//!    is never checked. Clause (b) still covers any source that has made a
//!    declaration, and in this slice's Specification both triggers and
//!    requirements come from required sources, so no flip is constructible
//!    today — but a Specification whose witnesses can come from a non-required
//!    source would need clause (c) widened.
//! 8. **(New.)** A Binding may be emitted twice: `Provisional` at hot
//!    settlement, `Authoritative` at cold settlement. This is a *derived*
//!    property of the evidence contract rather than a stipulation, so #16's
//!    store and #18's periphery may rely on it: key on Binding, expect an
//!    authority upgrade, and **never expect an outcome change**. #18 checked
//!    and preserved the Violate-class contract; the refusal envelope
//!    (`Satisfy → Inconclusive`) is the adjudicated exception and is not a
//!    Violate flip. #20 owns any renegotiation that would let cold `Violate`
//!    over hot `Satisfy`. That guarantee needs the contract to cover
//!    **every** field of `SourceEvidence`, not only the completeness bounds
//!    — a restated `tier` or `gaps` would otherwise flip an emitted outcome
//!    — which is why clause (a) refuses a restatement as well as a
//!    regression. The escape hatch is a new `SequenceEpoch`; the reasoning
//!    is scoped to a single epoch by construction.
//! 9. **(Inherited, #10; closed by #19, with a residue.)** Verdicts were
//!    emitted with a constant Filter epoch `0` and no activation barrier, both
//!    of which ADR-0017 requires. Both halves now land:
//!    `assurance.filter_epochs` records the epoch each source's own
//!    completeness signal acknowledged, and
//!    [`specification::PrecedenceSpecification::with_activation_barrier`] plus
//!    [`settlement::BarrierState`] make a warming Specification incapable of
//!    an authoritative Verdict. The residue is that **nothing declares a
//!    barrier yet**: `sentinel_compile`'s `compile_one` emits an empty
//!    `activation_watermarks` and there is no control plane to populate it, so
//!    the mechanism is complete and reachable from a valid Event Filter while
//!    the deployment is not. #19 stays open for it.
//! 11. **(New.)** A warming Specification can emit a `Provisional Violate`,
//!     and that is deliberate rather than an oversight. "Missing evidence is
//!     never a pass" is asymmetric: it governs passes, and a Violate is the
//!     other direction. ADR-0017's "Shadow **or** Inconclusive" is a
//!     disjunction and Shadow is what this is; and `CONTEXT.md` makes Shadow
//!     the mechanism for measuring false-positive rate before Promotion, so a
//!     barrier-induced false Violate **is** the false positive Shadow exists to
//!     surface. It is not enforceable — a Shadow Specification cannot appear in
//!     an unresolved Probe decision — and limitation 3 already flags that the
//!     model accepts such a Verdict carrying no witness. Attribution is
//!     `Violate ∧ Provisional ∧ PENDING` on the typed discriminator, a
//!     conservative overestimate of "caused by a stale Filter."
//! 12. **(New.)** The barrier makes a source's acknowledged Filter epoch
//!     load bearing for *authority*, which reaches further than the evidence
//!     contract does. Within one `SequenceEpoch` the gap is closed:
//!     [`evidence::MonotonicityBreach::RegressedFilterEpoch`] refuses a
//!     rollback, because otherwise a source re-declaring a lower epoch flipped
//!     its barrier cleared → pending and
//!     [`evaluator::Evaluator::advance`] re-emitted `Authoritative →
//!     Provisional`, which `sentinel-store` refuses outright.
//!
//!     **Across a `SequenceEpoch` it is not closed, and the residue is
//!     stated rather than implied.** A source restarting into a new epoch with
//!     a stale acknowledged Filter epoch is admissible — that is clause (a)'s
//!     documented escape hatch, and it is the same hatch tier, capabilities and
//!     gaps use. Executed: the restart itself re-emits nothing (the gate closes,
//!     `advance` returns `Step::Pending`), but a subsequent
//!     [`evaluator::Evaluator::seal`] emits `Provisional` where an
//!     `Authoritative` was already emitted. That sits inside territory this
//!     crate already scopes out — a barrier-free control over the same restart
//!     seals a *different outcome* for the same Binding, which limitation 8 and
//!     [`evaluator`]'s "scoped to a single epoch by construction" already
//!     disclose — so the barrier adds an authority axis to a case that was
//!     already outside the contract's guarantee rather than opening a new one.
//!     #19's control-plane follow-on owns the repair, because deciding whether a
//!     a restarted source may un-acknowledge an epoch is an acknowledgement
//!     question, not an evaluator one.
//!
//!     **Retirement removes the protection limitation 12 rests on, and the
//!     replacement is named here rather than left to be inferred (#110).** "The
//!     gate closes, so `advance` re-emits nothing" holds only for a Binding
//!     whose emission needs an open gate, and a retired Binding's does not. The
//!     replacement is the retired arm of the evaluator's emission key: one bit,
//!     joined with the last emission, so it advances `false -> true` at most
//!     once and is never written back — an `Authoritative -> Provisional`
//!     regression therefore re-emits nothing by construction rather than by the
//!     gate happening to be shut. The residue is unchanged and is
//!     `sentinel-store`'s: it refuses *authority* regressions with a conflict,
//!     while recording outcome changes as new revisions rather than refusing
//!     them.
//! 13. **(New, #110.)** A retired **guard-declaring** Specification never
//!     upgrades. `Evaluator::declare_queries` runs only inside the gate-open
//!     branch, and deliberately so: [`facts::FactLedger::declare`] is
//!     idempotent by `(binding, guard)`, so a floor snapshotted behind a closed
//!     gate would be permanently lower than the hot path's and a cold answer
//!     would settle a conjunct it never saw. A retired Binding behind a closed
//!     gate therefore has no query declared, stays `PENDING` / `Provisional`,
//!     and reaches `Authoritative` only if the gate later opens. Nothing ships
//!     broken — CAT-1 declares no guards — and #18 owns the cold path's own
//!     retirement horizon (ADR-0021's O-21a).
//! 14. **(New, #110.)** Evidence observed **after** a retirement horizon is out
//!     of the retired Slice and is **not** superseded. It is counted and
//!     disclosed in the Verdict's prefix-uncertainty detail rather than
//!     discarded silently, which is the honest half; the dishonest half would
//!     be claiming a later Verdict supersedes the earlier one, and none of
//!     supersession's machinery exists. #20 and ADR-0021's O-11 own Amendments.
//! 15. **(New, #110 / #210.)** The O-03 reason is **unstructured**.
//!     [`sentinel_model::InconclusiveReason`] is proto-generated and
//!     `just generate` is a networked maintenance boundary no gate crosses
//!     (ADR-0024), so retirement rides `Uncertainty.detail` as free text under
//!     `PrefixUncertainty`. Two consequences follow rather than being
//!     discovered: #21 and #35 cannot consume the signal programmatically, and
//!     a retired envelope is not byte-identical to any non-retirement path.
//!     `INCONCLUSIVE_REASON_RETIRED` = 6 is **deferred** to the
//!     `sentinel.catalogue.v1` proto trip (with #21/#35 consumers); this PR
//!     deliberately does not mint the enum value.
//! 16. **(New, #110.)** **Shadow's measurement disappears for a retired
//!     Binding.** Limitation 11 makes a barrier-induced false `Violate` exactly
//!     the false positive Shadow exists to surface, and the retirement pin
//!     removes it — while `sentinel-candidate`'s replay classifier counts
//!     `VerdictOutcome` values, so retiring a Binding reclassifies the Candidate
//!     replay evidence it contributes to. Whether a Shadow evaluation should
//!     honour retirement at all is #19/#21's decision, not this crate's.
//! 17. **(New, #110.)** **The Verdict surface and the enforcement surface can
//!     disagree about the same Binding.** [`decision::decide`]'s rows 9–12 read
//!     gate-open, covering Watermarks and the raw member set — none of which
//!     retirement touches, and none of which it *can* touch, because the
//!     lifecycle lives outside [`slice::SliceIndex`] and the seam has no
//!     accessor to it. So `decide()` can `Deny` a Binding whose Verdict this
//!     crate has pinned to `Inconclusive`. That is behaviour rather than an
//!     oversight: making retirement a `Defer` input is #22/#26's decision, and
//!     taking it here would be settling enforcement semantics as a side effect
//!     of a liveness feature.
//! 18. **(New, #210.)** **S4 SagaShop E2E residual (AC3 honesty B).** CAT-3's
//!     evaluator and catalogue halves are closed by synthetic pins
//!     (`existence_decision_table`, `existence_evaluator`, compile census /
//!     Filter / IR tests). A full open/required/close+`Retire` walk against a
//!     named `testdata/sagashop/…` capture for corpus scenario S4 is not
//!     claimed here — recorded also in `docs/PRD.md` O-03 and
//!     `docs/research/m0-evidence-corpus.md` P-D.
//! 19. **(New, #173.)** CAT-5 `aggregation.count-within` is declared as a
//!     per-trigger formula —
//!     `AG(trigger -> count_within(lookback, member) < threshold)` — but the
//!     cold count answer is a **per-Binding** fact today: a cold query is keyed
//!     by `(Binding, guard)` rather than by trigger Event, and
//!     [`aggregate::AggregateSpecification`] observes the count once per Binding
//!     then reuses that observation for every trigger. A Binding with five
//!     triggers therefore receives one count answer, not five, so lookbacks that
//!     would end at different triggers are indistinguishable. Documented on
//!     [`aggregate::AggregateObservation`]; disclosed here so an auditor does
//!     not have to rediscover it. Not a silent pass — incomplete lookbacks still
//!     never `Satisfy` — but the Verdict is shaped like a per-trigger claim while
//!     resting on a per-Binding fact.
//!
#![allow(clippy::doc_markdown)] // Product names (SagaShop, OpenTelemetry) read as prose here.

pub mod absence;
pub mod aggregate;
pub mod cost;
pub mod decision;
pub mod evaluator;
pub mod evidence;
pub mod existence;
pub mod facts;
pub mod frontier;
pub mod precedence;
pub mod settlement;
pub mod slice;
pub mod specification;
pub mod taint;
pub mod verdict;

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use sentinel_model::{Event, SourceEvidence, SourceIdentity, VerdictError};

use crate::evaluator::{Evaluator, Input};
use crate::evidence::NonMonotonicEvidence;
use crate::facts::ColdGuardId;
use crate::frontier::{CausalIndexError, FrontierError};
use crate::slice::{Binding, RetirementBreach};
use crate::specification::SpecificationIdentity;
use crate::verdict::BindingVerdict;

/// Failure evaluating a Specification against observed evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CoreError {
    /// Two deliveries made contradictory claims about one identity or slot.
    Delivery(FrontierError),
    /// The declared causal edges admit no partial order.
    Causality(CausalIndexError),
    /// The assembled Verdict violated a model invariant.
    Verdict(VerdictError),
    /// An input contradicted what the same source already declared.
    NonMonotonicEvidence(NonMonotonicEvidence),
    /// The batch path was asked to evaluate a Specification declaring guards.
    FactsRequired {
        /// The guards the batch path has no supplier for.
        guards: BTreeSet<ColdGuardId>,
    },
    /// A response echoed a query this evaluator never issued.
    FactResponseMismatch {
        /// The Binding the response claimed to answer for.
        binding: Binding,
        /// The guard the response claimed to answer for.
        guard: ColdGuardId,
    },
    /// No required source supplied both delivered Events and declared evidence.
    ///
    /// Evidence Coverage is empty while at least one Binding exists, so no
    /// Assurance Envelope is constructible. **That is `sentinel_model`'s
    /// ruling, not a preference of this crate's.** Two of the model's own rules
    /// close the path, and neither is `sentinel-core`'s to relax:
    ///
    /// 1. `AssuranceValidator::check` (`model/src/verdict.rs:681-696`) admits
    ///    no Verdict at *any* authority whose `hot_path` is not `SETTLED` — the
    ///    `Authoritative` arm rejects it as `AuthoritativeTierUnsettled`, the
    ///    `Provisional` arm as `ProvisionalTierMismatch`. There is no third
    ///    authority to retreat to.
    /// 2. `AssuranceValidator::validate_settlement_watermarks`
    ///    (`model/src/verdict.rs:912-915`) rejects a `SETTLED` tier carrying an
    ///    empty settlement Watermark set as `SettlementWatermarkMismatch`.
    ///
    /// `hot_settlement_watermarks` is built entirely from
    /// [`evidence::Coverage::covered`], so with no coverage rule 2 fires and
    /// rule 1 forbids stepping around it. The `hot_path: SETTLED` literal in
    /// [`crate::verdict`]'s envelope constructor is **downstream of rule 1, not
    /// the cause of this refusal**: making it conditional does not open a path,
    /// it only trades `SettlementWatermarkMismatch` for
    /// `ProvisionalTierMismatch`. The single remaining escape — inventing a
    /// Watermark for a source that declared nothing, in the field
    /// `validate_decisive_event` bounds witnesses with — needs a clock this
    /// crate does not hold (ADR-0012) and would resolve uncertainty by
    /// fabrication. Returning the error is the only answer that neither
    /// fabricates evidence nor passes — **missing evidence is never a pass**.
    ///
    /// `undeclared` is the actionable half of the diagnosis: those sources
    /// delivered Events, so the driver reached
    /// [`evidence::EvidenceLedger::record_delivery`] for them and never reached
    /// [`evidence::EvidenceLedger::record_source`]. Sources in `required` but
    /// not in `undeclared` were simply silent.
    ///
    /// *Partial* coverage is not this error. A Verdict whose required sources
    /// are only partly covered is constructible, and the Core builds it: an
    /// `Inconclusive` / `PrefixUncertainty` / `Provisional` Verdict whose
    /// envelope names the missing sources. Uncertainty is preserved there
    /// rather than refused. Nor is an *empty* required-source set this error:
    /// that is [`CoreError::SpecificationRequiresNoSource`].
    NoEvidenceCoverage {
        /// The Specification's required sources, at least one, none covered.
        required: BTreeSet<SourceIdentity>,
        /// Required sources that delivered Events but declared no evidence.
        undeclared: BTreeSet<SourceIdentity>,
    },
    /// The Specification declares no required source at all.
    ///
    /// A **Specification-validity defect, not an evidence condition**, and the
    /// two must not be conflated. [`CoreError::NoEvidenceCoverage`] says the
    /// deployment did not supply what the Specification asked for; this says
    /// the Specification asked for nothing. Reporting the second as the first
    /// would name a deployment fault — with an empty set of sources to
    /// instrument — for a defect no amount of evidence can repair.
    ///
    /// No Verdict is constructible either way, and for the same two model rules
    /// [`CoreError::NoEvidenceCoverage`] sets out: `hot_settlement_watermarks`
    /// is drawn from the covered required sources, so an empty required set
    /// yields an empty Watermark set. The difference is the repair. Here it is
    /// to fix the Specification's text.
    ///
    /// [`specification::PrecedenceSpecification::new`] stays infallible: a
    /// Specification is `sentinel-compile`'s artifact and rejecting a
    /// vacuous one belongs at Promotion, not in the evaluator. The evaluator's
    /// job is to refuse it by name rather than to guess what was meant.
    SpecificationRequiresNoSource {
        /// The Specification whose required-source set is empty.
        specification: SpecificationIdentity,
    },
    /// A retirement signal breached the admissibility contract (ADR-0021, O-03).
    ///
    /// An error rather than a degrade, on the grounds [`evidence`]'s clauses
    /// (a), (b) and (c) already establish: the Core is being told something
    /// contradictory about a Scope, and quietly absorbing it would resolve
    /// uncertainty by fabrication. The refusal is **recoverable** — every clause
    /// is checked before any state is mutated, so the evaluator stays usable, an
    /// empty [`evaluator::Evaluator::resume`] succeeds and
    /// [`evaluator::Evaluator::seal`] still returns Verdicts.
    ///
    /// [`slice::RetirementBreach::BindingAlreadyEmitted`] is the one a driver is
    /// most likely to meet, and it has a published guard,
    /// [`evaluator::Evaluator::is_emitted`]. Refusing it is what stops
    /// retirement overwriting an emitted Verdict with an `Inconclusive` — an
    /// outcome retraction with no Amendment record, which limitation 12 and
    /// ADR-0021's O-11 both forbid.
    Retirement(RetirementBreach),
}

impl Display for CoreError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Delivery(error) => write!(formatter, "delivery is not admissible: {error}"),
            Self::Causality(error) => write!(formatter, "causal edges are contradictory: {error}"),
            Self::Verdict(error) => write!(formatter, "verdict is not constructible: {error}"),
            Self::NonMonotonicEvidence(error) => {
                write!(formatter, "evidence is not monotone: {error}")
            }
            Self::FactsRequired { guards } => write!(
                formatter,
                "the batch path has no fact supplier for {} declared cold guard(s)",
                guards.len()
            ),
            Self::FactResponseMismatch { binding, guard } => write!(
                formatter,
                "no query was issued for binding {binding} and guard {}",
                guard.id()
            ),
            Self::NoEvidenceCoverage {
                required,
                undeclared,
            } => write!(
                formatter,
                "no Evidence Coverage: none of {} required source(s) supplied \
                 both Events and declared evidence, and {} of them delivered \
                 Events without declaring any",
                required.len(),
                undeclared.len()
            ),
            Self::SpecificationRequiresNoSource { specification } => write!(
                formatter,
                "Specification {} version {} declares no required source, so no \
                 Verdict it produces can carry a settlement Watermark",
                specification.id(),
                specification.version()
            ),
            Self::Retirement(breach) => write!(
                formatter,
                "the retirement signal is not admissible: {}",
                breach.detail()
            ),
        }
    }
}

impl Error for CoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Delivery(error) => Some(error),
            Self::Causality(error) => Some(error),
            Self::Verdict(error) => Some(error),
            Self::NonMonotonicEvidence(error) => Some(error),
            Self::FactsRequired { .. }
            | Self::FactResponseMismatch { .. }
            | Self::NoEvidenceCoverage { .. }
            | Self::SpecificationRequiresNoSource { .. }
            | Self::Retirement(_) => None,
        }
    }
}

impl From<FrontierError> for CoreError {
    fn from(error: FrontierError) -> Self {
        Self::Delivery(error)
    }
}

impl From<CausalIndexError> for CoreError {
    fn from(error: CausalIndexError) -> Self {
        Self::Causality(error)
    }
}

impl From<VerdictError> for CoreError {
    fn from(error: VerdictError) -> Self {
        Self::Verdict(error)
    }
}

impl From<NonMonotonicEvidence> for CoreError {
    fn from(error: NonMonotonicEvidence) -> Self {
        Self::NonMonotonicEvidence(error)
    }
}

/// Evaluates one Specification over a closed evidence batch, one Verdict per Binding.
///
/// This is a **driver over the stepping API**, not a second implementation:
/// every `Input::Event` in arrival order, then every `Input::Source`, then
/// [`Evaluator::seal`]. The returned vector is sorted by Binding, because
/// per-Verdict byte identity does not imply vector identity.
///
/// The Events-then-Sources order is deliberate. It is byte-identical either way
/// — the gate reads final ledger state, not arrival order — but feeding
/// evidence about completeness *after* the Events it describes is the order the
/// monotonicity contract is written for, and a Sources-first driver would have
/// to special-case its own first phase against contract clause (b).
///
/// # Errors
///
/// Returns [`CoreError::FactsRequired`] for a Specification declaring any cold
/// guard: the batch path has no fact supplier, and it must not silently drop a
/// declared guard. Otherwise returns [`CoreError`] for a delivery conflict, a
/// contradictory causal cycle, non-monotone evidence, or an envelope the model
/// rejects.
///
/// Returns [`CoreError::NoEvidenceCoverage`] when `events` delivers for the
/// required sources but `sources` declares a `SourceEvidence` for none of them:
/// this driver seals, and sealing is the one path that reaches the Assurance
/// Envelope with no Evidence Coverage to build it from. Returns
/// [`CoreError::SpecificationRequiresNoSource`] for a Specification whose
/// required-source set is empty.
pub fn evaluate(
    mut evaluator: Evaluator,
    events: &[Event],
    sources: &BTreeMap<SourceIdentity, SourceEvidence>,
) -> Result<Vec<BindingVerdict>, CoreError> {
    if !evaluator.declared_cold_guards().is_empty() {
        return Err(CoreError::FactsRequired {
            guards: evaluator.declared_cold_guards().clone(),
        });
    }

    for event in events {
        evaluator.step(Input::event(event.clone()))?;
    }
    for evidence in sources.values() {
        evaluator.step(Input::source(evidence.clone()))?;
    }
    Ok(evaluator.seal()?.into_verdicts())
}
