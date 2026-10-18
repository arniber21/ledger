//! The enforcement decision seam: ADR-0023's synchronous decision call.
//!
//! [`decide`] answers a Probe's [`DecideRequest`] from the monitor state the
//! Verification Core already holds: one [`DecisionRecord`] — action, declared
//! fail mode, unresolved reason — per matched active-enforcement
//! Specification, plus the aggregate action, assemblable to
//! [`raw_probe::DecideResponse`] with [`DecisionOutcome::to_wire`]. This is
//! issue #22's entire surface: the seam, not the server. `api/` is greenfield
//! (#162/#166), no control plane publishes a Filter yet, no store holds
//! monitor state, and serving the `SentinelDecisionService` RPC is the thin
//! periphery #26 and #166 will host. `sdk/dotnet`'s transport notes point at
//! #22 for the endpoint; the recorded decision is that the endpoint is
//! deliberately **not** landed here, and the seam is.
//!
//! This is not `taint::decide`, which answers a different question (CAT-4's
//! taint table) in a different module. Nothing here shares state or
//! vocabulary with it.
//!
//! # The decision is a per-action causal guard, not a read of the Verdict
//!
//! The cheap design — clone an [`Evaluator`], feed it the pending action,
//! read the Binding's Verdict, map `Violate → Deny / Satisfy → Permit /
//! Inconclusive → Defer` — is wrong, and ADR-0023's own cost row says why: "a
//! false block is a production incident". A Binding-level Verdict folds every
//! trigger in the Slice (`BindingOutcome::Violate` dominates), so an order
//! that already shipped once without approval would deny a second, approved
//! action. The Verdict remains the auditable post-hoc judgement over the
//! Binding; the decision answers *may this action proceed*, computed as the
//! CPL guard ADR-0023 names as the precedent: the requirement witness test
//! for **this** pending trigger, over the causally-visible frontier, under
//! the monitor's own evidence rules.
//!
//! # The ordered decision table
//!
//! Per matched Specification, first match wins:
//!
//! | # | condition | decision |
//! |---|---|---|
//! | 0 | effective budget = min(Transport Budget at this hop, declared budget) is zero | Defer(TIMEOUT) |
//! | 1 | the asked action's Event is already delivered | error `AlreadyDelivered` |
//! | 2 | the trigger selector does not match the asked action | Permit |
//! | 3 | the trigger matches but the action binds no Binding | Defer(EVIDENCE_GAP) |
//! | 4 | the activation barrier is still pending | Defer(EVIDENCE_GAP) |
//! | 5 | the Specification declares cold guards, or queries are outstanding | Defer(EVIDENCE_GAP) |
//! | 6 | a delivered, admitted, anchor-tier requirement witness is in the action's declared causal past | Permit |
//! | 7 | the action's declared past is incomplete (a dangling predecessor) | Defer(EVIDENCE_GAP) |
//! | 8 | a matching requirement is delivered but not ordered before the action | Defer(OBSERVATIONAL_UNCERTAINTY) |
//! | 9 | the evidence gate is closed (G1/G1b/G2/G3/G4/G5) | Defer(EVIDENCE_GAP) |
//! | 10 | the gate is open but Watermarks do not cover the epochs | Defer(PREFIX_UNCERTAINTY) |
//! | 11 | absence is certifiable: anchor-tier Events of the requirement's kind exist in the Binding's Slice | Deny |
//! | 12 | otherwise — absence not anchor-witnessed | Defer(EVIDENCE_GAP) |
//!
//! Rows 9–12 are the monitor's Violated row verbatim, down to the
//! anchor-downgrade rule: a deny is an **absence certification** and demands
//! everything the model demands of an authoritative Violate. Missing
//! evidence is never a pass; missing evidence is never a block either.
//!
//! `EPISTEMIC_UNCERTAINTY` is deliberately absent from [`DeferReason`]: a
//! hot-path guard still cannot defer for that reason. A terminal cold
//! [`crate::facts::GuardConclusion::Refused`] does populate
//! `InconclusiveReason::EpistemicUncertainty` on the Verdict (Finding 11 /
//! `REFUSED_GUARD_DETAIL`). That is not a `DeferReason` arm.
//!
//! # Three deliberate divergences from the Verdict surface
//!
//! The guard reuses the monitor's semantics — the gate's disjuncts, the tier
//! rule, the anchor-downgrade rule, the declared-edge relation — and
//! diverges in exactly three named places, each a design decision, each
//! pinned by a test:
//!
//! 1. **A Permit demands an anchor-tier witness; the monitor's Discharged
//!    does not.** ADR-0023's fourth gate is about the Events anchoring the
//!    *decision*: an approval the monitored application could forge is a
//!    self-approval, not a discharge. A contributing-tier-only witness falls
//!    through to row 12 (Defer) — under an accepted fail-closed contract
//!    that is defer-then-deny, and that is the recorded cost of not trusting
//!    a forgeable approval.
//! 2. **A Permit does not wait for the gate; the monitor demotes everything
//!    when gated.** G5 fires mid-stream constantly; requiring an open gate
//!    for permits would defer nearly every decision and make enforcement
//!    useless. The gate exists to certify **absences**; that is the only
//!    place the table demands it (rows 9–11).
//! 3. **A Permit may rest on a path through a delivered-but-held
//!    intermediary.** The index resolves over the delivered set, held Events
//!    included, so a real witness reached through a held intermediary is a
//!    settled positive fact even while G4 is closed. The monitor given the
//!    same evidence would hold the trigger and emit nothing; the guard
//!    permits on the positive evidence. Recorded as intended: the guard
//!    reads the same declared causal edges the Verdict surface reads, and a
//!    producer that fabricates a parent edge to borrow a real approval
//!    attacks the substrate (ADR-0019's scope), not this seam.
//!
//! # The asked action is the request's producer
//!
//! Identity, kind, attributes and declared causal predecessors are producer
//! facts, and [`DecideRequest::producer`] is the only representation of the
//! asked action this seam holds. There is no adapter obligation here and no
//! second [`Event`] argument to echo against: disagreement is
//! unrepresentable. `ObservationTime`, Data Class and `SourceAssurance`
//! are not on the producer and cannot be supplied at this seam — they are
//! receiver-owned fields of a delivered Event, which is what rows 6, 8 and
//! 11 still read. Reconstructing a dummy Event for the asked action would
//! invent those three fields, and that invention is what this fold makes
//! impossible.
//!
//! # Cross-checks, all loud errors
//!
//! Two ADR-0031-class transposition hazards are built into any
//! map-keyed signature, and both are refused here: the evaluators-map key
//! must equal the enclosed evaluator's own Specification identity
//! ([`DecisionError::EvaluatorIdentityMismatch`]), and the request's
//! acknowledged epoch must equal the handed Filter's epoch
//! ([`DecisionError::FilterEpochMismatch`]). The entry's
//! `target_filter_epoch` equality is **not** re-checked:
//! `EventFilter::try_from` already enforces it, and duplicating that check
//! would be the second implementation ADR-0027 forbids.
//!
//! A third refusal answers monitor state the monitor itself will not
//! settle: an [`Evaluator`] whose delivered edges admit no partial order
//! ([`DecisionError::CausalContradiction`]) is refused in the same
//! per-entry loop, because its `advance()` returns `Pending` on exactly
//! that condition and its `seal()` raises it, so a row-6 Permit over the
//! stale pre-contradiction index would answer a question the Verdict
//! surface refuses to pose. This is a cross-check, not a fourth
//! divergence: the three named monitor-semantics divergences above stand;
//! a contradictory edge set is a wiring or producer defect, the same
//! class of bug as a mis-keyed evaluators map.
//!
//! # Budgets
//!
//! The effective budget is the minimum of the caller-supplied Transport
//! Budget remainder — the periphery's measured at-this-hop value, **zero
//! allowed**, because each hop decrements it with its own monotonic clock
//! and the Core holds no clock (ADR-0012, purity-gated) — and the
//! Specification's declared Enforcement Latency Budget. The budget used
//! rides the typed record so the export surface (#25/#36) will not re-derive
//! it.
//!
//! # Row 4's duplication, named
//!
//! The wire entry's activation barrier and the core Specification's
//! `activation_barrier` are two representations of one rollout fact. No
//! production path stamps the core-side one today (`TryFrom<&PrecedenceIr>`
//! drops it; `compile_one` emits empty Watermarks — the residue core's
//! limitation 9 records), so row 4 is test-only until #166; the seam reads
//! the core-side representation and names #166 as the owner of the
//! duplication.
//!
//! # What is deliberately not here
//!
//! No transport and no `SentinelDecisionService` server (#26/#166); no
//! idempotency or replay — the request carries an `idempotency_key`, and
//! remembering answers is a store concern (#166), so the seam is pure and
//! stateless; no decision-latency metric and no fail-mode-trigger-rate
//! counter (ADR-0014's enforcement metrics are #25/#36's); no proto change
//! (#164 owns the next wire wave); compile's `FailMode::Open` /
//! budget-`None` default is still cited rather than duplicated
//! (`compile/src/filter.rs:813-814`); no Verdict invariants are touched —
//! the seam emits `DecisionRecord`s, not Verdicts, so `model`'s
//! decisive-witness and Provisional rules are out of scope.

#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use sentinel_model::wire::model::v1 as raw_model;
use sentinel_model::wire::probe::v1 as raw_probe;
use sentinel_model::{
    AvailabilityContract, DecideRequest, Event, EventFilter, EventId, FilterEpoch, ProducerEvent,
    SourceTier,
};

use crate::evaluator::Evaluator;
use crate::evidence::EvidenceGate;
use crate::frontier::{CausalIndexError, HypotheticalReach};
use crate::settlement::BarrierState;
use crate::slice::Binding;
use crate::specification::{
    PrecedenceSpecification, SpecificationId, SpecificationIdentity, SpecificationVersion,
};

/// The action Sentinel returns for one Specification (ADR-0023:
/// permit/deny/defer).
///
/// The derived ordering is the aggregate's dominance order:
/// `Permit < Defer(_) < Deny`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum Decision {
    /// The action may proceed.
    Permit,
    /// Sentinel cannot answer yet; the Probe applies its declared fail mode.
    Defer(DeferReason),
    /// The action may not proceed.
    Deny,
}

/// Why a decision deferred, in the Core's vocabulary; mapped exhaustively (no
/// wildcard arm) onto [`raw_probe::UnresolvedReason`] at the wire boundary.
///
/// `EPISTEMIC_UNCERTAINTY` is deliberately absent: a hot-path guard still
/// cannot defer for that reason. A terminal cold `Refused` populates
/// `InconclusiveReason::EpistemicUncertainty` on the Verdict; that is not
/// a [`DeferReason`] arm.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum DeferReason {
    /// The effective budget was exhausted at this hop.
    Timeout,
    /// Evidence the table demands is missing or unclassifiable.
    EvidenceGap,
    /// Required-source Watermarks have not caught up with the emitted epoch.
    PrefixUncertainty,
    /// A requirement exists but cannot be ordered against the action.
    ///
    /// The label is the safe direction's name: the action is not in the
    /// index, so a requirement Concurrent with the action and one in the
    /// action's *future* are indistinguishable here, and both defer rather
    /// than block.
    ObservationalUncertainty,
}

impl DeferReason {
    /// Maps onto the wire reason, exhaustively.
    fn to_wire(self) -> raw_probe::UnresolvedReason {
        match self {
            Self::Timeout => raw_probe::UnresolvedReason::Timeout,
            Self::EvidenceGap => raw_probe::UnresolvedReason::EvidenceGap,
            Self::PrefixUncertainty => raw_probe::UnresolvedReason::PrefixUncertainty,
            Self::ObservationalUncertainty => raw_probe::UnresolvedReason::ObservationalUncertainty,
        }
    }
}

impl Decision {
    /// Maps onto the wire action, exhaustively.
    fn to_wire(self) -> raw_probe::DecisionAction {
        match self {
            Self::Permit => raw_probe::DecisionAction::Permit,
            Self::Deny => raw_probe::DecisionAction::Deny,
            Self::Defer(_) => raw_probe::DecisionAction::Defer,
        }
    }
}

/// One matched Specification's decision, before wire assembly.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionRecord {
    identity: SpecificationIdentity,
    decision: Decision,
    fail_mode: raw_model::FailMode,
    effective_budget_nanoseconds: u64,
}

impl DecisionRecord {
    /// Returns the identity of the Specification this record decides for.
    #[must_use]
    pub const fn identity(&self) -> &SpecificationIdentity {
        &self.identity
    }

    /// Returns the action for this Specification.
    #[must_use]
    pub const fn decision(&self) -> Decision {
        self.decision
    }

    /// Returns the Filter entry's **declared** fail mode, echoed verbatim.
    ///
    /// The Probe's unresolved fold rejects any record whose fail mode drifted
    /// from its active Filter entry, so this echo is a wire contract, not a
    /// convenience.
    #[must_use]
    pub const fn fail_mode(&self) -> raw_model::FailMode {
        self.fail_mode
    }

    /// Returns the budget this decision was computed under: the minimum of
    /// the measured Transport Budget at this hop and the declared Enforcement
    /// Latency Budget (ADR-0023: the budget rides the decision's record).
    #[must_use]
    pub const fn effective_budget_nanoseconds(&self) -> u64 {
        self.effective_budget_nanoseconds
    }
}

/// The whole answer: aggregate action plus per-Specification records.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionOutcome {
    request_id: String,
    aggregate: Decision,
    records: Vec<DecisionRecord>,
}

impl DecisionOutcome {
    /// Returns the echoed request identifier.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    /// Returns the aggregate action.
    ///
    /// Deny if any record denies; else Defer if any defers; else Permit —
    /// deny-wins mirrors fail-closed-wins. A deferring aggregate carries the
    /// first deferring record's reason, which is deterministic because the
    /// records are sorted by identity.
    #[must_use]
    pub const fn aggregate(&self) -> Decision {
        self.aggregate
    }

    /// Returns the per-Specification records, sorted by identity.
    #[must_use]
    pub fn records(&self) -> &[DecisionRecord] {
        &self.records
    }

    /// Assembles the wire response.
    ///
    /// Deterministic by construction: the aggregate action, one
    /// [`raw_probe::SpecificationDecision`] per record in identity order —
    /// identity, action, the declared fail mode verbatim, and
    /// `unresolved_reason` present exactly on `Defer`. `aggregate_unresolved`
    /// governs the **all-DEFER** response (it rejects any non-DEFER record);
    /// a mixed aggregate — the designed outcome whenever the matched set
    /// splits — is consumed action-only by the six live Probe gates, which
    /// carry the records informationally.
    #[must_use]
    pub fn to_wire(&self) -> raw_probe::DecideResponse {
        raw_probe::DecideResponse {
            request_id: self.request_id.clone(),
            action: self.aggregate.to_wire() as i32,
            specifications: self
                .records
                .iter()
                .map(|record| raw_probe::SpecificationDecision {
                    specification_id: record.identity.id().to_owned(),
                    specification_version: record.identity.version().to_owned(),
                    action: record.decision.to_wire() as i32,
                    fail_mode: record.fail_mode as i32,
                    unresolved_reason: match record.decision {
                        Decision::Defer(reason) => Some(reason.to_wire() as i32),
                        Decision::Permit | Decision::Deny => None,
                    },
                })
                .collect(),
        }
    }
}

/// Why [`decide`] refused to answer.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecisionError {
    /// The asked action's Event is already in the Trace: the decision is moot.
    AlreadyDelivered {
        /// The delivered Event identity.
        event: EventId,
    },
    /// A matched enforcing Specification has no monitor state — a wiring
    /// bug, loud.
    MissingEvaluator {
        /// The Specification the map had no evaluator for.
        specification: SpecificationIdentity,
    },
    /// The evaluators map is keyed by a different identity than the Evaluator
    /// it hands over — the ADR-0031 transposition the map invites. A mis-
    /// keyed map would decide one Specification's question with another's
    /// selectors, silently.
    EvaluatorIdentityMismatch {
        /// The map key that disagrees with the evaluator it holds.
        key: SpecificationIdentity,
    },
    /// The matched Specification's monitor state carries delivered causal
    /// edges that admit no partial order — a wiring or producer defect of
    /// exactly the class [`DecisionError::AlreadyDelivered`] answers. The
    /// monitor's own `advance()` returns `Pending` on that condition and
    /// `seal()` raises it, so the seam refuses too, rather than answer over
    /// the stale pre-contradiction index.
    CausalContradiction {
        /// The contradictory cycle the monitor state carries.
        contradiction: CausalIndexError,
    },
    /// The request's acknowledged Filter epoch differs from the Filter handed
    /// with it — the matched set would be computed against the wrong epoch.
    FilterEpochMismatch {
        /// The epoch the request acknowledged.
        request: FilterEpoch,
        /// The epoch the handed Filter carries.
        filter: FilterEpoch,
    },
    /// A fail-closed declaration with no accepted availability contract — a
    /// promotion-time misconfiguration, refused loudly.
    ///
    /// The refusal's cost, stated exactly: it surfaces as a transport-level
    /// error, never a synthesized DEFER, and the Probe answers it with its
    /// aggregate accepted fail mode — so a request co-matched by a correctly
    /// contracted CLOSED Specification DENIES on this error path. That is the
    /// accepted cost of refusing loudly instead of serving a downgraded
    /// record the wire contract forbids (`aggregate_unresolved` rejects any
    /// fail-mode mismatch). The durable fix — refusing to *publish* a
    /// fail-closed entry without a persisted contract — is Promotion's
    /// (#166), which `AvailabilityContract` already arms.
    FailClosedWithoutContract {
        /// The fail-closed Specification no contract covers.
        specification: SpecificationIdentity,
    },
}

impl Display for DecisionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyDelivered { event } => {
                write!(formatter, "the asked action is already delivered: {event}")
            }
            Self::MissingEvaluator { specification } => write!(
                formatter,
                "matched Specification {} has no monitor state",
                specification.id()
            ),
            Self::EvaluatorIdentityMismatch { key } => write!(
                formatter,
                "evaluators map key {} holds another Specification's monitor state",
                key.id()
            ),
            Self::CausalContradiction { contradiction } => write!(
                formatter,
                "the monitor state's delivered edges admit no partial order: {contradiction}"
            ),
            Self::FilterEpochMismatch { request, filter } => write!(
                formatter,
                "request acknowledges Filter epoch {} but the handed Filter is epoch {}",
                request.value(),
                filter.value()
            ),
            Self::FailClosedWithoutContract { specification } => write!(
                formatter,
                "fail-closed Specification {} has no accepted availability contract",
                specification.id()
            ),
        }
    }
}

impl Error for DecisionError {}

/// Answers one Probe decision request from the monitor state the Core holds.
///
/// The asked action is [`DecideRequest::producer`]: kind, identity,
/// attributes and declared predecessors are read from there. The matched
/// set is recomputed exactly as the Probe computes it (`specSelects` and
/// `isEnforceable`, the same four conditions): Filter entries whose
/// `event_match` carries the asked action's kind, delivered ask-and-block,
/// evaluated under enforcement, active. Shadow and Warming entries are
/// never matched and never decided. An empty matched set yields `Permit`
/// with no records — the mirror of the Probe gate's ship-async no-op.
///
/// # Errors
///
/// Returns [`DecisionError::FilterEpochMismatch`] when the request's
/// acknowledged epoch differs from the handed Filter's; per matched entry,
/// [`DecisionError::MissingEvaluator`] when no monitor state exists,
/// [`DecisionError::EvaluatorIdentityMismatch`] when the map key disagrees
/// with the evaluator's own Specification identity, or when the evaluator
/// is CAT-4 (not a `decide()` input; ADR-0023 gate 3),
/// [`DecisionError::CausalContradiction`] when the entry's monitor state
/// carries delivered edges that admit no partial order, and
/// [`DecisionError::FailClosedWithoutContract`] when a fail-closed entry has
/// no accepted [`AvailabilityContract`]. During the per-Specification table,
/// [`DecisionError::AlreadyDelivered`] when the asked action is already in
/// the Trace.
///
/// # Panics
///
/// Never on an evidence gap. Matched entries are active-enforcement entries,
/// which the Filter constructor refuses to build without a declared fail mode
/// and a nonzero latency budget; `identity_of` expects those Filter-validated
/// `specification_id` / `specification_version` strings (the same
/// `require_text` rule as `string_identifier!`). Those are caller-contract
/// breaks on a validated [`EventFilter`], matching the fail-mode expect, not
/// evidence panics.
#[must_use = "a decision is the load-bearing output of enforcement"]
pub fn decide(
    request: &DecideRequest,
    transport_budget_nanoseconds: u64,
    filter: &EventFilter,
    evaluators: &BTreeMap<SpecificationIdentity, Evaluator>,
    accepted: &BTreeSet<AvailabilityContract>,
) -> Result<DecisionOutcome, DecisionError> {
    if request.filter_epoch() != filter.epoch() {
        return Err(DecisionError::FilterEpochMismatch {
            request: request.filter_epoch(),
            filter: filter.epoch(),
        });
    }

    let producer = request.producer();

    // The canonically ordered entries keep the records sorted by identity,
    // which is what makes `to_wire` byte-deterministic.
    let matched: Vec<&raw_model::SpecificationFilter> = filter
        .specifications()
        .iter()
        .filter(|entry| is_matched(entry, producer.kind().as_str()))
        .collect();
    if matched.is_empty() {
        return Ok(DecisionOutcome {
            request_id: request.request_id().to_owned(),
            aggregate: Decision::Permit,
            records: Vec::new(),
        });
    }

    // The cross-checks run for every matched entry before any record is
    // computed — the two ADR-0031 transposition hazards and the causal
    // contradiction the monitor itself refuses to settle — so none of them
    // can leak into a served answer.
    for entry in &matched {
        let identity = identity_of(entry);
        let Some(evaluator) = evaluators.get(&identity) else {
            return Err(DecisionError::MissingEvaluator {
                specification: identity,
            });
        };
        if evaluator
            .specification()
            .is_none_or(|specification| specification.identity() != &identity)
        {
            return Err(DecisionError::EvaluatorIdentityMismatch { key: identity });
        }
        if let Some(contradiction) = evaluator.causal_contradiction() {
            return Err(DecisionError::CausalContradiction {
                contradiction: contradiction.clone(),
            });
        }
        let fail_mode =
            raw_model::FailMode::try_from(entry.fail_mode).expect("validated fail mode");
        if fail_mode == raw_model::FailMode::Closed
            && !accepted.iter().any(|contract| {
                contract.accepts(
                    entry.specification_id.as_str(),
                    entry.specification_version.as_str(),
                )
            })
        {
            return Err(DecisionError::FailClosedWithoutContract {
                specification: identity,
            });
        }
    }

    let mut records = Vec::with_capacity(matched.len());
    for entry in &matched {
        let identity = identity_of(entry);
        let evaluator = &evaluators[&identity];
        let specification =
            evaluator
                .specification()
                .ok_or_else(|| DecisionError::EvaluatorIdentityMismatch {
                    key: identity.clone(),
                })?;
        let decision = decide_one(
            &producer,
            transport_budget_nanoseconds,
            entry,
            specification,
            evaluator,
        )?;
        records.push(DecisionRecord {
            identity,
            decision,
            fail_mode: raw_model::FailMode::try_from(entry.fail_mode).expect("validated fail mode"),
            effective_budget_nanoseconds: transport_budget_nanoseconds.min(
                entry
                    .latency_budget_nanoseconds
                    .expect("matched active enforcement declares a budget"),
            ),
        });
    }

    Ok(DecisionOutcome {
        request_id: request.request_id().to_owned(),
        aggregate: aggregate_of(&records),
        records,
    })
}

/// The Probe's own matched-set conditions, recomputed on the Sentinel side.
///
/// All four are mirror-completeness against `specSelects` +
/// `isEnforceable`: on a validated `EventFilter`, `EvaluationMode::Enforce`
/// already implies `DeliveryMode::AskAndBlock` (enforcement validation
/// demands it) and `Readiness::Active` (Warming demands Shadow evaluation),
/// so the delivery and readiness conditions can never disagree with the
/// Probe — the constructor's refusals are what pin those two, and
/// `shadow_and_warming_entries_are_never_matched` the kind and evaluation
/// ones.
fn is_matched(entry: &raw_model::SpecificationFilter, kind: &str) -> bool {
    entry.event_match.as_ref().is_some_and(|event_match| {
        event_match
            .event_kinds
            .iter()
            .any(|candidate| candidate == kind)
            && event_match.delivery_mode == raw_model::DeliveryMode::AskAndBlock as i32
    }) && entry.evaluation_mode == raw_model::EvaluationMode::Enforce as i32
        && entry.readiness == raw_model::Readiness::Active as i32
}

fn identity_of(entry: &raw_model::SpecificationFilter) -> SpecificationIdentity {
    SpecificationIdentity::new(
        SpecificationId::new(entry.specification_id.as_str())
            .expect("EventFilter::try_from already require_text'd specification_id"),
        SpecificationVersion::new(entry.specification_version.as_str())
            .expect("EventFilter::try_from already require_text'd specification_version"),
    )
}

/// Deny if any record denies; else Defer if any defers; else Permit.
///
/// A deferring aggregate carries the first deferring record's reason, which
/// is deterministic because the records are sorted by identity.
fn aggregate_of(records: &[DecisionRecord]) -> Decision {
    let mut aggregate = Decision::Permit;
    for record in records {
        aggregate = match (aggregate, record.decision()) {
            (Decision::Deny, _) | (_, Decision::Deny) => Decision::Deny,
            (Decision::Defer(reason), _) | (_, Decision::Defer(reason)) => Decision::Defer(reason),
            (Decision::Permit, Decision::Permit) => Decision::Permit,
        };
    }
    aggregate
}

/// The ordered decision table for one matched Specification.
fn decide_one(
    producer: &ProducerEvent,
    transport_budget_nanoseconds: u64,
    entry: &raw_model::SpecificationFilter,
    specification: &PrecedenceSpecification,
    evaluator: &Evaluator,
) -> Result<Decision, DecisionError> {
    let declared_budget = entry
        .latency_budget_nanoseconds
        .expect("matched active enforcement declares a budget");

    // Row 0: the only clock-free budget rule a pure seam can have. The
    // caller supplies the measured remainder — zero allowed — because each
    // hop decrements with its own monotonic clock and the Core holds none.
    if transport_budget_nanoseconds.min(declared_budget) == 0 {
        return Ok(Decision::Defer(DeferReason::Timeout));
    }

    // Row 1: a delivered action is a moot question, not an uncertain one.
    if evaluator.frontier().event(&producer.id()).is_some() {
        return Err(DecisionError::AlreadyDelivered {
            event: producer.id(),
        });
    }

    // Row 2: a trigger that does not govern this action is deliberate
    // non-coverage — the monitor's answer to it is silence.
    if !specification.trigger().matches_producer(producer) {
        return Ok(Decision::Permit);
    }

    // Row 3: a governed action that binds no Binding attribute is a defect
    // (the #144 census's `NoBindings`), and unevaluable is never a pass.
    let Some(binding) = specification.binding_of_producer(producer) else {
        return Ok(Decision::Defer(DeferReason::EvidenceGap));
    };
    let binding = Binding::new(binding);

    // Row 4: a permit is a pass, and a warming Specification's evidence is
    // what it does not yet trust (ADR-0017).
    let coverage = evaluator
        .ledger()
        .coverage(specification.required_source_identities());
    if BarrierState::of(specification, &coverage).is_pending() {
        return Ok(Decision::Defer(DeferReason::EvidenceGap));
    }

    // Row 5: a cold guard is a conjunct, and a hot permit would be a pass on
    // an unproven conjunct (ADR-0021, one-sided). An Enforce entry carrying
    // guards is a promotion-time contradiction; the seam degrades it to
    // Defer and the Probe's timeout applies the fail mode.
    if !specification.cold_guards().is_empty() || !evaluator.outstanding().is_empty() {
        return Ok(Decision::Defer(DeferReason::EvidenceGap));
    }

    let declared: Vec<EventId> = producer.causal_predecessors().into_iter().collect();
    let index = evaluator.causal_index();
    let frontier = evaluator.frontier();

    // Row 6: the witness test. Anchor tier demanded (divergence 1), gate
    // not waited for (divergence 2), held intermediaries admitted on the
    // path (divergence 3). Slice membership supplies both "delivered" and
    // "admitted": the SliceIndex is fed only the Frontier's newly-admitted
    // Events, so a delivered-but-held Event is never a slice member.
    let slice: Vec<&Event> = evaluator
        .slices()
        .members(&binding)
        .into_iter()
        .flatten()
        .filter_map(|id| frontier.event(id))
        .collect();
    let witness = slice.iter().any(|event| {
        specification.requirement().matches(event)
            && evaluator.ledger().tier_of(&event.source()) == Some(SourceTier::Anchor)
            && index.reaches_from_declared(&event.id(), &declared) == HypotheticalReach::Reaches
    });
    if witness {
        return Ok(Decision::Permit);
    }

    // Row 7: the action's own identity is never in the index (row 1 saw to
    // that), so probing with it yields exactly the declared past's
    // completeness.
    if index.reaches_from_declared(&producer.id(), &declared) == HypotheticalReach::PastIncomplete {
        return Ok(Decision::Defer(DeferReason::EvidenceGap));
    }

    // Row 8: a matching requirement that is delivered but cannot be ordered
    // before the action. `NotReaches` cannot distinguish Concurrent from
    // After, so a requirement in the action's future also defers here — the
    // safe direction's label (see `DeferReason::ObservationalUncertainty`).
    let unordered = frontier.delivered_events().any(|event| {
        specification
            .binding_of(event)
            .is_some_and(|value| Binding::new(value) == binding)
            && specification.requirement().matches(event)
            && index.reaches_from_declared(&event.id(), &declared) == HypotheticalReach::NotReaches
    });
    if unordered {
        return Ok(Decision::Defer(DeferReason::ObservationalUncertainty));
    }

    // Rows 9–12: the monitor's Violated row, verbatim.
    let gate = EvidenceGate::evaluate(
        specification.required_source_identities(),
        evaluator.ledger(),
        frontier,
    );

    // Row 9: the gate certifies absences; here and only here is it demanded.
    if !gate.is_open() {
        return Ok(Decision::Defer(DeferReason::EvidenceGap));
    }

    // Row 10: mid-stream Watermarks leave absence uncertifiable.
    if !gate.watermarks_cover_epochs() {
        return Ok(Decision::Defer(DeferReason::PrefixUncertainty));
    }

    // Row 11: the 4.3 anchor-downgrade rule — an authoritative absence needs
    // an anchor-tier Event of the requirement's kind in the same Binding's
    // Slice to be a witnessed fact rather than silence.
    let anchored = slice.iter().any(|event| {
        specification.requirement().matches_kind(event)
            && evaluator.ledger().tier_of(&event.source()) == Some(SourceTier::Anchor)
    });
    if anchored {
        return Ok(Decision::Deny);
    }

    // Row 12: absence with no anchor witness is uncertifiable, never a Deny.
    Ok(Decision::Defer(DeferReason::EvidenceGap))
}
