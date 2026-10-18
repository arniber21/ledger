//! The tier-settlement rule: authority, `cold_path`, and cold Watermarks.
//!
//! One pure function of `(coverage complete, guards declared, every query
//! settled)`. It is named `settlement` and not `composition` because **nothing
//! is composed**: #12 builds exactly one of ADR-0021's rows, the provisional
//! one ("either pending → provisional"), and zero outcome-composition rows. A
//! module called `composition.rs` is precisely where a future contributor would
//! put back the row that was cut for being unconstructible against `model`.
//!
//! ```text
//! cold_path = NOT_REQUIRED   iff  no guards declared AND coverage complete
//!           = SETTLED        iff  guards declared AND every query settled AND coverage complete
//!           = PENDING        otherwise
//!
//! authority = Authoritative  iff  cold_path != PENDING and activation_barrier != PENDING
//!           = Provisional    iff  cold_path == PENDING or  activation_barrier == PENDING
//!
//! cold_settlement_watermarks = clamp((union of settling answers) ∩ required,
//!                                     evidence)                          iff SETTLED
//!                            = empty                                     otherwise
//! ```
//!
//! # The ceiling clamp, and why it is not neutral
//!
//! The cold path may answer *ahead* of the Core: it reads a store that has
//! seen sequences the Core's own evidence has not. `sentinel_model`'s
//! `validate_settlement_reached` requires `evidence[s] >= cold[s]` for every
//! source, so an answer above the Core's evidence Watermark used to come back
//! out of `Evaluator::resume` as `CoreError::Verdict` — a periphery input
//! reported as a Core bug, and worse, one that wedged the evaluator (#108).
//!
//! So the recorded coordinate is clamped down to the Core's own evidence
//! Watermark. Within one epoch this is monotone-sound in the only direction
//! that matters: an answer computed at `(0, 17)` did, necessarily, see
//! everything through `(0, 16)`. What it is **not** is neutral. Clamping
//! records less coverage than the periphery actually achieved, and
//! `crate`'s admitted-limitation 5 names exactly this shape — *"the model
//! rewards under-reporting cold settlement"*. A cold path that answers behind
//! the Core is indistinguishable, in the Verdict, from one that answers ahead
//! of it. That is a real weakness in the incentive structure, chosen here
//! deliberately because the alternative is either fabricating coverage or
//! surfacing a periphery input as a Core bug, and both are worse.
//!
//! Across epochs the clamp would be fabrication rather than under-reporting —
//! `(1, 0)` recorded as `(0, 16)` asserts coverage of a sequence space the
//! answer never touched. `FactResponse::settles` therefore refuses to settle an
//! answer whose epoch differs from its query's floor, so the common case
//! re-issues and never arrives here at all. That is not the whole of it,
//! though: the floor is a *snapshot*, and the ledger moves on. An answer can
//! sit in its floor's epoch and still sit in an epoch the source has since left
//! by the time a Verdict is assembled. `clamp_to_evidence` carries the argument
//! for which of its arms the [`crate::evaluator::Evaluator`] can reach, and why
//! the one it cannot reach today stays anyway.
//!
//! # Why `cold_path` reads both inputs and not coverage alone
//!
//! A flat `hot = SETTLED, cold = NOT_REQUIRED` makes the partial-coverage case
//! **unconstructible**: `Verdict::try_from` requires `Provisional` to carry
//! `hot == SETTLED` *and* `cold == PENDING` exactly, so a guard-free
//! Specification at incomplete coverage would be rejected as
//! `ProvisionalTierMismatch`. For a Specification declaring no guard this
//! function therefore reproduces #10's coverage-driven behaviour exactly, byte
//! for byte, rather than replacing it.
//!
//! # Two constraints `sentinel_model` imposes, both load-bearing
//!
//! - An Authoritative `cold = SETTLED` needs `cold_settlement_watermarks` to
//!   name **exactly** the required-source set: `validate_settlement_watermarks`
//!   applies `require_authoritative_coverage`, which is set *equality*. So the
//!   union of settling answers is restricted to the required set **before** it
//!   reaches the envelope. Without that, a response naming
//!   `required ∪ {anything else}` would carry the extra source in and return
//!   `AuthoritativeCoverageIncomplete` as a `CoreError::Verdict` — a periphery
//!   input reported as a Core bug, which the degrade-never-error discipline
//!   exists to prevent.
//! - `Pending` and `NotRequired` need `cold_settlement_watermarks` to be
//!   **exactly empty**. A `Provisional` Verdict is therefore structurally
//!   incapable of recording that a guard answered.
//!
//! # Soundness note for #18
//!
//! `authority = Authoritative iff cold_path != PENDING and activation_barrier
//! != PENDING` is sound **only because `hot_path` is unconditionally
//! `Settled`**; `Verdict::try_from` rejects Authoritative when `hot != SETTLED`
//! or the barrier is pending. A guard-free complete-coverage warming Verdict is
//! `Provisional` + `cold = NOT_REQUIRED` + `activation_barrier = PENDING`, so
//! the pre-#164 biconditional (`Authoritative iff cold != PENDING`) is false.
//! #18 introducing real hot-path settlement states must revisit this line. The
//! model will reject the mistake, but loudly and late.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use sentinel_model::{
    ActivationBarrier, BarrierCleared, BarrierPending, SourceEvidence, SourceIdentity,
    SourceLivenessFreshness, TierSettlement, VerdictAuthority,
};

use crate::absence::AbsenceSpecification;
use crate::aggregate::AggregateSpecification;
use crate::evidence::Coverage;
use crate::existence::ExistenceSpecification;
use crate::facts::{EvidenceWatermarks, SettledWatermarks, WatermarkPoint};
use crate::specification::PrecedenceSpecification;
use crate::taint::TaintSpecification;

/// Whether one Specification has cleared the activation barrier it declared.
///
/// # Two separate guarantees, because one of them used to be overstated
///
/// **(1) Not constructible outside this crate.** The single field is private and holds a
/// private enum, so [`BarrierState::of`] is the only way a caller anywhere else obtains a
/// value, and `of` derives it from a Specification and its Evidence Coverage. There is no
/// `Default`, no `From`, and no deserializer.
///
/// **(2) Not *substitutable* into the settlement rule — and this is the half that had to
/// be built rather than described.** Guarantee (1) alone is worth much less than it looks,
/// because it says nothing about *which* Specification a state was derived from. While
/// `settle` took `&BarrierState` as a parameter, a downstream caller could derive
/// `NotDeclared` from a three-line decoy Specification and pass it when settling a warming
/// one — obtaining `Authoritative` from a Specification behind an unclear barrier, with
/// every type-level rule respected. That was executed against an earlier revision of this
/// file and it worked. Raising the cost of a forgery from "name a variant" to "write a
/// decoy Specification" is not closing it, and a doc comment claiming otherwise is exactly
/// the prose-instead-of-a-type failure ADR-0031 clause 1 exists to refuse.
///
/// So the parameter is gone. `settle` takes the `&PrecedenceSpecification` and derives
/// the barrier state itself; the rule's table lives in a **private** `settle_derived`, and
/// nothing outside this crate can reach it. There is no longer a barrier argument that can
/// disagree with the Specification being settled, because there is no barrier argument.
/// `Settlement::barrier` hands the derived state back so [`crate::verdict`] reads the
/// one `settle` computed instead of receiving a second copy that could differ — the same
/// transposition hazard, one call frame further on.
///
/// That fix (#19) moved the unchecked pairing one call frame down rather than removing
/// it: the `crate::verdict` builders were public and took a `Settlement`, an
/// `EvidenceGate` and an assessment beside a Specification, checking none of them against
/// it. #304 closed that pairing by sealing `Settlement` — `settle` and its siblings are
/// crate-private — and making the builders crate-private. No public function that issues a
/// Verdict or produces a `Settlement` now accepts a Settlement, Evidence Gate or
/// assessment alongside a Specification it was not derived from; the guards are listed
/// under `Settlement`'s `# Sealed (#304)` section. A Verdict still binds its
/// Specification by identity only, which #304 does not address: see #309.
///
/// What a caller *can* still do is supply a [`Coverage`] built from evidence it fabricated.
/// That is not laundering and it is not this type's problem: it is supplying false
/// evidence to a pure function, which is visible, and it moves in the conservative
/// direction whenever it is merely *incomplete* — a source missing from `covered` is
/// [`sentinel_model::SourceBarrierEvidence::NoEvidence`] and holds the barrier.
///
/// # Why a private-field struct and not `#[non_exhaustive]`
///
/// `#[non_exhaustive]` on the enum blocks nothing — a downstream crate can still name a
/// unit variant. `#[non_exhaustive]` per variant blocks construction but **also** blocks
/// downstream tuple-variant matching, so a consumer could not read the [`BarrierCleared`]
/// proof back out. The private-field struct blocks construction and keeps the accessors.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BarrierState {
    inner: BarrierInner,
}

/// The three states, private so that only [`BarrierState::of`] can name them.
#[derive(Clone, Debug, Eq, PartialEq)]
enum BarrierInner {
    /// The Specification declares no activation barrier.
    NotDeclared,
    /// Every required source acknowledged the target epoch and reached its Watermark.
    Cleared(BarrierCleared),
    /// At least one required source has not.
    Pending(BarrierPending),
}

impl BarrierState {
    /// Derives the barrier state from a Specification and its Evidence Coverage.
    ///
    /// Public because it is a **query**, not a capability: it answers "is this
    /// Specification still warming against this evidence?", which is what issue #21 needs
    /// to read. It confers nothing, because no public function accepts a
    /// [`BarrierState`] — `settle` derives its own, and is itself crate-private, so the
    /// `Settlement` that carries a barrier state cannot be obtained outside this crate
    /// either (#304; see `Settlement`'s `# Sealed (#304)` section).
    ///
    /// The evidence is drawn from [`Coverage::covered`] rather than from the whole ledger,
    /// which is the conservative direction: a required source that delivered Events and
    /// declared no [`SourceEvidence`] is not covered, so it supplies no completeness
    /// signal to the barrier and is answered
    /// [`sentinel_model::SourceBarrierEvidence::NoEvidence`]. **Missing evidence is never
    /// a pass**, and a barrier that cleared on half of Evidence Coverage would make a
    /// warming Specification authoritative on exactly the evidence it is warming to
    /// acquire.
    #[must_use]
    pub fn of(specification: &PrecedenceSpecification, coverage: &Coverage<'_>) -> Self {
        Self::of_barrier(specification.activation_barrier(), coverage)
    }

    /// Derives the barrier state from a CAT-4 Specification and its Evidence Coverage.
    ///
    /// Sibling of [`BarrierState::of`]: substitution-safety requires the barrier
    /// to come from the same value that is settled, so a taint Specification
    /// cannot be passed to [`BarrierState::of`].
    #[must_use]
    pub fn of_taint(specification: &TaintSpecification, coverage: &Coverage<'_>) -> Self {
        Self::of_barrier(specification.activation_barrier(), coverage)
    }

    /// Derives the barrier state from a CAT-2 Specification and its Evidence Coverage.
    ///
    /// Sibling of [`BarrierState::of_taint`].
    #[must_use]
    pub fn of_absence(specification: &AbsenceSpecification, coverage: &Coverage<'_>) -> Self {
        Self::of_barrier(specification.activation_barrier(), coverage)
    }

    /// Derives the barrier state from a CAT-5 Specification and its Evidence Coverage.
    ///
    /// Sibling of [`BarrierState::of_absence`].
    #[must_use]
    pub fn of_aggregate(specification: &AggregateSpecification, coverage: &Coverage<'_>) -> Self {
        Self::of_barrier(specification.activation_barrier(), coverage)
    }

    fn of_barrier(barrier: Option<&ActivationBarrier>, coverage: &Coverage<'_>) -> Self {
        let Some(barrier) = barrier else {
            return Self {
                inner: BarrierInner::NotDeclared,
            };
        };
        let evidence: BTreeMap<SourceIdentity, &SourceEvidence> = coverage
            .covered()
            .iter()
            .map(|(source, record)| (source.clone(), *record))
            .collect();
        Self {
            inner: match barrier.clear(&evidence) {
                Ok(cleared) => BarrierInner::Cleared(cleared),
                Err(pending) => BarrierInner::Pending(pending),
            },
        }
    }

    /// Derives barrier state under caller-supplied source-liveness floors (O-12 / #163).
    ///
    /// Sibling of [`Self::of`]: activation admission is unchanged, then each admitted
    /// source is checked against [`SourceLivenessFreshness`]. Missing evidence is still
    /// never a pass. Pure — the floors are data; this reads no clock.
    #[must_use]
    pub fn of_within(
        specification: &PrecedenceSpecification,
        coverage: &Coverage<'_>,
        liveness: &SourceLivenessFreshness,
    ) -> Self {
        Self::of_barrier_within(specification.activation_barrier(), coverage, liveness)
    }

    /// CAT-4 sibling of [`Self::of_within`].
    #[must_use]
    pub fn of_taint_within(
        specification: &TaintSpecification,
        coverage: &Coverage<'_>,
        liveness: &SourceLivenessFreshness,
    ) -> Self {
        Self::of_barrier_within(specification.activation_barrier(), coverage, liveness)
    }

    /// CAT-2 sibling of [`Self::of_within`].
    #[must_use]
    pub fn of_absence_within(
        specification: &AbsenceSpecification,
        coverage: &Coverage<'_>,
        liveness: &SourceLivenessFreshness,
    ) -> Self {
        Self::of_barrier_within(specification.activation_barrier(), coverage, liveness)
    }

    /// CAT-5 sibling of [`Self::of_within`].
    #[must_use]
    pub fn of_aggregate_within(
        specification: &AggregateSpecification,
        coverage: &Coverage<'_>,
        liveness: &SourceLivenessFreshness,
    ) -> Self {
        Self::of_barrier_within(specification.activation_barrier(), coverage, liveness)
    }

    /// Derives the barrier state from a CAT-3 Specification and its Coverage.
    #[must_use]
    pub fn of_existence(specification: &ExistenceSpecification, coverage: &Coverage<'_>) -> Self {
        Self::of_barrier(specification.activation_barrier(), coverage)
    }

    /// Derives the barrier state under caller-supplied source-liveness floors.
    #[must_use]
    pub fn of_existence_within(
        specification: &ExistenceSpecification,
        coverage: &Coverage<'_>,
        liveness: &SourceLivenessFreshness,
    ) -> Self {
        Self::of_barrier_within(specification.activation_barrier(), coverage, liveness)
    }

    fn of_barrier_within(
        barrier: Option<&ActivationBarrier>,
        coverage: &Coverage<'_>,
        liveness: &SourceLivenessFreshness,
    ) -> Self {
        let Some(barrier) = barrier else {
            return Self {
                inner: BarrierInner::NotDeclared,
            };
        };
        let evidence: BTreeMap<SourceIdentity, &SourceEvidence> = coverage
            .covered()
            .iter()
            .map(|(source, record)| (source.clone(), *record))
            .collect();
        Self {
            inner: match barrier.clear_within(&evidence, liveness) {
                Ok(cleared) => BarrierInner::Cleared(cleared),
                Err(pending) => BarrierInner::Pending(pending),
            },
        }
    }

    /// Returns whether a declared barrier is still holding.
    ///
    /// The predicate `settle` caps authority on, and the only one of the four that is
    /// about *suppression*. Its negation covers both "no barrier declared" and "declared
    /// and cleared", which is correct: neither suppresses anything.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(self.inner, BarrierInner::Pending(_))
    }

    /// Returns whether a barrier was declared **and** every required source cleared it.
    ///
    /// Strictly narrower than `!is_pending()`: a Specification declaring no barrier is
    /// neither cleared nor pending, and conflating the two would let a consumer read
    /// "cleared" as evidence that a rollout completed when none was ever declared.
    #[must_use]
    pub const fn is_cleared(&self) -> bool {
        matches!(self.inner, BarrierInner::Cleared(_))
    }

    /// Returns whether the Specification declares an activation barrier at all.
    #[must_use]
    pub const fn is_declared(&self) -> bool {
        !matches!(self.inner, BarrierInner::NotDeclared)
    }

    /// Returns the clearing proof, when the barrier cleared.
    #[must_use]
    pub const fn cleared(&self) -> Option<&BarrierCleared> {
        match &self.inner {
            BarrierInner::Cleared(cleared) => Some(cleared),
            BarrierInner::NotDeclared | BarrierInner::Pending(_) => None,
        }
    }

    /// Returns what is still holding the barrier, when it is pending.
    #[must_use]
    pub const fn pending(&self) -> Option<&BarrierPending> {
        match &self.inner {
            BarrierInner::Pending(pending) => Some(pending),
            BarrierInner::NotDeclared | BarrierInner::Cleared(_) => None,
        }
    }
}

/// The settled tier state one Verdict is emitted under.
///
/// # Sealed (#304)
///
/// A `Settlement` cannot be obtained outside this crate. The type itself is
/// `pub(crate)`, its fields are private, it is `#[non_exhaustive]`, it has no
/// `Default`, `From` or deserializer, and its only producers — `settle`, `settle_within`, their per-family siblings and the private
/// `settle_derived` — are crate-private. So a function that takes a `&Settlement` cannot
/// be called from outside `sentinel-core`, whatever its name and whether or not it is
/// `pub`. The one caller is [`crate::evaluator::Evaluator`], which derives the
/// Settlement, the `EvidenceGate` and the family's assessment from the single
/// Specification it holds and passes them to the crate-private `crate::verdict` builders.
///
/// What this establishes, and no more: **no public function that issues a Verdict or
/// produces a `Settlement` accepts a Settlement, Evidence Gate or assessment alongside a
/// Specification it was not derived from.** The public per-family `assess` / `observe`
/// queries do take an unchecked `EvidenceGate` beside a Specification; nothing public
/// consumes what they return.
///
/// **The seal rests on one condition: no public function returns a `Settlement`.**
/// Because the type is `pub(crate)`, rustc's `private_interfaces` lint checks that
/// condition. A reachable `pub` function whose signature names `Settlement` — an
/// accessor such as `BindingVerdict::settlement()` added for diagnostics, or a builder or
/// `settle*` made `pub` again — and a public field of that type are reported as "type
/// `Settlement` is more private than the item". `just lint` runs Clippy with
/// `-D warnings`, so there it is an error; in a plain `cargo build` it is only a warning.
/// Verified by mutation: with `settle` made `pub`, Clippy under `-D warnings` fails on it.
///
/// **What it does not close.** A Verdict names its Specification by
/// `SpecificationIdentity` only. A Specification with the same identity and different
/// content — its cold guards stripped, for example — still yields the same Verdict through
/// the public `Evaluator` and through `decide`, whose `EvaluatorIdentityMismatch` check
/// also compares identities only. That is #309.
///
/// ## The guards
///
/// One `compile_fail,E0603` doctest per sealed item, each turning red when its item is made
/// `pub` again. One block per item, because a single block naming every item stays red
/// while any one of them is private. Two caveats, both verified on the Devbox pin:
///
/// - **Write `compile_fail` first.** Stable rustdoc reads `E0603,compile_fail` as a
///   non-Rust block and silently does not run it.
/// - **Stable rustdoc does not check the error code.** A block naming an item that no
///   longer exists fails with `E0425` and still passes, so a rename turns its block
///   vacuous; the code is documentation, enforced only on nightly. The `#[cfg(test)]`
///   backstops in this module and in `crate::verdict` name every listed item, so a rename
///   or removal breaks the `--lib` build and points back here. They guard names, not
///   privacy.
///
/// The ten producers, which carry the seal:
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::settlement::settle;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::settlement::settle_within;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::settlement::settle_taint;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::settlement::settle_taint_within;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::settlement::settle_absence;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::settlement::settle_absence_within;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::settlement::settle_aggregate;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::settlement::settle_aggregate_within;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::settlement::settle_existence;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::settlement::settle_existence_within;
/// ```
///
/// The seven Verdict builders, defence in depth (each takes a `&Settlement` and so is
/// uncallable from outside even if made `pub`):
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::verdict::build;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::verdict::build_retired;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::verdict::build_taint;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::verdict::build_absence;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::verdict::build_aggregate;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::verdict::build_existence;
/// ```
///
/// ```compile_fail,E0603
/// let _ = sentinel_core::verdict::build_existence_retired;
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub(crate) struct Settlement {
    authority: VerdictAuthority,
    cold_path: TierSettlement,
    cold_settlement_watermarks: BTreeMap<SourceIdentity, WatermarkPoint>,
    barrier: BarrierState,
}

impl Settlement {
    /// Returns the authority the Verdict is emitted under.
    #[must_use]
    pub const fn authority(&self) -> VerdictAuthority {
        self.authority
    }

    /// Returns the cold path's tier settlement.
    #[must_use]
    pub const fn cold_path(&self) -> TierSettlement {
        self.cold_path
    }

    /// Returns the Watermarks the cold path settled at, restricted to required.
    #[must_use]
    pub const fn cold_settlement_watermarks(&self) -> &BTreeMap<SourceIdentity, WatermarkPoint> {
        &self.cold_settlement_watermarks
    }

    /// Returns the activation-barrier state this settlement was computed under.
    ///
    /// Carried rather than recomputed. [`crate::verdict`] needs it to disclose a pending
    /// barrier in the Assurance Envelope, and handing it the value `settle` actually
    /// used is what makes the disclosure and the authority cap incapable of disagreeing.
    /// A second `&BarrierState` parameter alongside `&PrecedenceSpecification` — which is
    /// what this replaced — is #108's transposition defect with a different pair of
    /// arguments: two values whose correspondence is available and unchecked.
    #[must_use]
    pub const fn barrier(&self) -> &BarrierState {
        &self.barrier
    }
}

/// Applies the settlement rule for one Binding.
///
/// # Why this takes a Specification rather than the three facts it needs from one
///
/// `coverage_complete`, `guards_declared`, `required` and the activation-barrier state are
/// all derived from `specification` and `coverage` here, instead of being passed in
/// alongside them. That looks like a convenience and is not: it is what makes the four
/// **incapable of disagreeing with each other**.
///
/// While the barrier arrived as its own `&BarrierState` parameter, a downstream caller
/// could derive `NotDeclared` from a decoy Specification and pass it while settling a
/// warming one — real `required`, real coverage, laundered barrier, `Authoritative` out.
/// That was executed. Removing the parameter is the only fix that does not rest on a
/// promise: there is now exactly one Specification input, and everything Specification
/// shaped comes from it.
///
/// [`Settlement::barrier`] hands the derived state back, so `crate::verdict` reads the one
/// this function used rather than receiving a second copy alongside the same Specification.
///
/// The rule's own table lives in the private [`settle_derived`], which is where the tests
/// exercise it directly. That split is deliberate and is not a hole: `settle_derived` is
/// private, so the substitution above is unreachable from outside this crate, while the
/// table stays testable without standing up an [`crate::evidence::EvidenceLedger`] for
/// every row.
///
/// # Why this is crate-private (#304)
///
/// Removing the barrier parameter did not remove the unchecked pairing; it moved it one
/// call frame down. While this function and its siblings were public, a downstream caller
/// could settle a decoy Specification here and hand the [`Settlement`] to a public
/// `crate::verdict` builder beside the real one. So this function and its nine siblings
/// are crate-private: they are the only producers of a [`Settlement`], which therefore
/// cannot be obtained outside this crate, and any function taking one is uncallable from
/// outside whatever its name. [`Settlement`]'s `# Sealed (#304)` section states exactly
/// what that establishes, the one condition it rests on (checked by the
/// `private_interfaces` lint under `just lint`), and the identity-binding gap it does
/// not close (#309).
///
/// # The two Watermark maps
///
/// `settled_watermarks` is the union of the settling answers' `settled_at`, which this
/// function restricts to `required` and clamps to `evidence_watermarks` before it can
/// reach an envelope.
///
/// `evidence_watermarks` is the Core's own per-source Watermark — the same coordinate
/// `crate::verdict` writes into `assurance.watermarks` and
/// `assurance.hot_settlement_watermarks`. It is the ceiling because `sentinel_model` makes
/// it one.
///
/// One precondition, and it is the caller's: when `every_query_settled` is true,
/// `settled_watermarks` must name every source in `required`. The `Evaluator` gets that
/// for free — `FactResponse::settles` will not settle a query whose answer omits a
/// required source — and a debug build traps a caller that does not. A release build
/// degrades to `Provisional` instead of panicking, because this function's whole purpose
/// is to never take a periphery condition and make it someone's crash.
#[must_use]
pub(crate) fn settle(
    specification: &PrecedenceSpecification,
    coverage: &Coverage<'_>,
    every_query_settled: bool,
    settled_watermarks: &SettledWatermarks,
    evidence_watermarks: &EvidenceWatermarks,
) -> Settlement {
    settle_derived(
        BarrierState::of(specification, coverage),
        coverage.is_complete(),
        !specification.cold_guards().is_empty(),
        every_query_settled,
        settled_watermarks,
        evidence_watermarks,
        &specification.required_source_identities(),
    )
}

/// Settles a CAT-4 Specification, deriving the barrier from that same value.
///
/// Sibling of [`settle`]: do not reopen `settle(&BarrierState)` — that was the
/// barrier substitution #19 removed. Both paths call private [`settle_derived`].
#[must_use]
pub(crate) fn settle_taint(
    specification: &TaintSpecification,
    coverage: &Coverage<'_>,
    every_query_settled: bool,
    settled_watermarks: &SettledWatermarks,
    evidence_watermarks: &EvidenceWatermarks,
) -> Settlement {
    settle_derived(
        BarrierState::of_taint(specification, coverage),
        coverage.is_complete(),
        !specification.cold_guards().is_empty(),
        every_query_settled,
        settled_watermarks,
        evidence_watermarks,
        &specification.required_source_identities(),
    )
}

/// Settles a CAT-1 Specification under caller-supplied source-liveness floors.
///
/// Sibling of [`settle`]: uses [`BarrierState::of_within`] so a post-activation
/// silence is named on the Verdict rather than collapsed into missing evidence.
#[must_use]
pub(crate) fn settle_within(
    specification: &PrecedenceSpecification,
    coverage: &Coverage<'_>,
    every_query_settled: bool,
    settled_watermarks: &SettledWatermarks,
    evidence_watermarks: &EvidenceWatermarks,
    liveness: &SourceLivenessFreshness,
) -> Settlement {
    settle_derived(
        BarrierState::of_within(specification, coverage, liveness),
        coverage.is_complete(),
        !specification.cold_guards().is_empty(),
        every_query_settled,
        settled_watermarks,
        evidence_watermarks,
        &specification.required_source_identities(),
    )
}

/// Settles a CAT-4 Specification under caller-supplied source-liveness floors.
#[must_use]
pub(crate) fn settle_taint_within(
    specification: &TaintSpecification,
    coverage: &Coverage<'_>,
    every_query_settled: bool,
    settled_watermarks: &SettledWatermarks,
    evidence_watermarks: &EvidenceWatermarks,
    liveness: &SourceLivenessFreshness,
) -> Settlement {
    settle_derived(
        BarrierState::of_taint_within(specification, coverage, liveness),
        coverage.is_complete(),
        !specification.cold_guards().is_empty(),
        every_query_settled,
        settled_watermarks,
        evidence_watermarks,
        &specification.required_source_identities(),
    )
}

/// Settles a CAT-2 Specification, deriving the barrier from that same value.
#[must_use]
pub(crate) fn settle_absence(
    specification: &AbsenceSpecification,
    coverage: &Coverage<'_>,
    every_query_settled: bool,
    settled_watermarks: &SettledWatermarks,
    evidence_watermarks: &EvidenceWatermarks,
) -> Settlement {
    settle_derived(
        BarrierState::of_absence(specification, coverage),
        coverage.is_complete(),
        !specification.cold_guards().is_empty(),
        every_query_settled,
        settled_watermarks,
        evidence_watermarks,
        &specification.required_source_identities(),
    )
}

/// Settles a CAT-2 Specification under caller-supplied source-liveness floors.
#[must_use]
pub(crate) fn settle_absence_within(
    specification: &AbsenceSpecification,
    coverage: &Coverage<'_>,
    every_query_settled: bool,
    settled_watermarks: &SettledWatermarks,
    evidence_watermarks: &EvidenceWatermarks,
    liveness: &SourceLivenessFreshness,
) -> Settlement {
    settle_derived(
        BarrierState::of_absence_within(specification, coverage, liveness),
        coverage.is_complete(),
        !specification.cold_guards().is_empty(),
        every_query_settled,
        settled_watermarks,
        evidence_watermarks,
        &specification.required_source_identities(),
    )
}

/// Settles a CAT-5 Specification, deriving the barrier from that same value.
#[must_use]
pub(crate) fn settle_aggregate(
    specification: &AggregateSpecification,
    coverage: &Coverage<'_>,
    every_query_settled: bool,
    settled_watermarks: &SettledWatermarks,
    evidence_watermarks: &EvidenceWatermarks,
) -> Settlement {
    settle_derived(
        BarrierState::of_aggregate(specification, coverage),
        coverage.is_complete(),
        !specification.cold_guards().is_empty(),
        every_query_settled,
        settled_watermarks,
        evidence_watermarks,
        &specification.required_source_identities(),
    )
}

/// Settles a CAT-5 Specification under caller-supplied source-liveness floors.
#[must_use]
pub(crate) fn settle_aggregate_within(
    specification: &AggregateSpecification,
    coverage: &Coverage<'_>,
    every_query_settled: bool,
    settled_watermarks: &SettledWatermarks,
    evidence_watermarks: &EvidenceWatermarks,
    liveness: &SourceLivenessFreshness,
) -> Settlement {
    settle_derived(
        BarrierState::of_aggregate_within(specification, coverage, liveness),
        coverage.is_complete(),
        !specification.cold_guards().is_empty(),
        every_query_settled,
        settled_watermarks,
        evidence_watermarks,
        &specification.required_source_identities(),
    )
}

/// Settles a CAT-3 Specification, deriving the barrier from that same value.
#[must_use]
pub(crate) fn settle_existence(
    specification: &ExistenceSpecification,
    coverage: &Coverage<'_>,
    every_query_settled: bool,
    settled_watermarks: &SettledWatermarks,
    evidence_watermarks: &EvidenceWatermarks,
) -> Settlement {
    settle_derived(
        BarrierState::of_existence(specification, coverage),
        coverage.is_complete(),
        !specification.cold_guards().is_empty(),
        every_query_settled,
        settled_watermarks,
        evidence_watermarks,
        &specification.required_source_identities(),
    )
}

/// Settles a CAT-3 Specification under caller-supplied source-liveness floors.
#[must_use]
pub(crate) fn settle_existence_within(
    specification: &ExistenceSpecification,
    coverage: &Coverage<'_>,
    every_query_settled: bool,
    settled_watermarks: &SettledWatermarks,
    evidence_watermarks: &EvidenceWatermarks,
    liveness: &SourceLivenessFreshness,
) -> Settlement {
    settle_derived(
        BarrierState::of_existence_within(specification, coverage, liveness),
        coverage.is_complete(),
        !specification.cold_guards().is_empty(),
        every_query_settled,
        settled_watermarks,
        evidence_watermarks,
        &specification.required_source_identities(),
    )
}

/// The settlement table itself, over facts already reduced from one Specification.
///
/// Private. [`settle`], [`settle_taint`], [`settle_absence`],
/// [`settle_aggregate`] and [`settle_existence`] and their `_within` siblings
/// are the only ways in from the rest of this crate (all ten are themselves
/// crate-private since #304), and each derives every argument below from a
/// single Specification — which is the whole of the barrier-substitution fix. Keeping
/// the table as its own function is what lets the unit tests drive each row
/// without constructing an `EvidenceLedger` and an `Event` per row.
#[must_use]
fn settle_derived(
    barrier: BarrierState,
    coverage_complete: bool,
    guards_declared: bool,
    every_query_settled: bool,
    settled_watermarks: &SettledWatermarks,
    evidence_watermarks: &EvidenceWatermarks,
    required: &BTreeSet<SourceIdentity>,
) -> Settlement {
    let mut cold_path = if !coverage_complete {
        TierSettlement::Pending
    } else if !guards_declared {
        TierSettlement::NotRequired
    } else if every_query_settled {
        TierSettlement::Settled
    } else {
        TierSettlement::Pending
    };

    let mut cold_settlement_watermarks = BTreeMap::new();
    if cold_path == TierSettlement::Settled {
        // The clamp knowingly under-reports; see this module's header. When it
        // cannot under-report soundly it degrades the whole tier rather than
        // recording a coordinate the answers do not support, because the one
        // thing this function may never do is error.
        match clamp_to_evidence(settled_watermarks, evidence_watermarks, required) {
            Some(clamped) => cold_settlement_watermarks = clamped,
            None => cold_path = TierSettlement::Pending,
        }
    }

    // ADR-0006's activation barrier and ADR-0017's "cannot produce authoritative Verdicts
    // until all required sources acknowledge the epoch and the activation Watermark
    // passes" make Filter-epoch acknowledgement a precondition of *any* authoritative
    // Verdict. The barrier caps **authority only**. It does not force `cold_path =
    // Pending` or discard a cold coordinate the evidence may have reached — that was
    // limitation 10, deleted by #164. A warming, guard-free, complete-coverage Verdict
    // is Provisional + cold = NOT_REQUIRED + activation_barrier = PENDING, and keeps
    // cold_settlement_watermarks when the cold path actually settled.
    //
    // Deliberately **not** done here: suppressing the Verdict. A warming Specification may
    // still emit a `Provisional Violate`. "Missing evidence is never a pass" governs
    // passes; a Violate is the other direction, an over-call rather than a silent pass.
    // ADR-0017's "Shadow **or** Inconclusive" is a disjunction and Shadow is what this is.
    // And CONTEXT.md makes Shadow the mechanism for *measuring* false-positive rate before
    // Promotion -- a barrier-induced false Violate, where a source shipping under a stale
    // Event Filter never captured the approval Event and its absence reads as a violation,
    // **is** the false positive Shadow exists to surface. Suppressing it would delete the
    // measurement rather than make anything safer.

    // Read *after* the clamp: a degrade there is a degrade in authority too.
    let authority = if cold_path == TierSettlement::Pending || barrier.is_pending() {
        VerdictAuthority::Provisional
    } else {
        VerdictAuthority::Authoritative
    };

    Settlement {
        authority,
        cold_path,
        cold_settlement_watermarks,
        barrier,
    }
}

/// Restricts the settling union to `required` and clamps it to the evidence.
///
/// Returns `None` when no sound record of what the answers covered exists, and
/// the caller turns that into a degrade. None of the four exits is an error.
///
/// # Which exits the `Evaluator` can actually reach
///
/// Worth writing down, because the four look interchangeable and are not, and
/// because two of them carry soundness arguments that would read as dead prose
/// if their reachability were left to the reader to reconstruct.
///
/// **A required source the union never named** is unreachable from
/// [`crate::evaluator::Evaluator`]. `cold_path == Settled` implies
/// `every_query_settled`; every settled query was settled by a response
/// `FactResponse::settles` accepted; and that predicate's coverage half is
/// exactly "names every required source", over the same set passed here. So the
/// union covers `required` by construction. That makes it a broken caller
/// contract rather than an evidence condition, which is why it is asserted
/// rather than degraded quietly: a quiet degrade would turn a real invariant
/// break into a Verdict that is merely weaker than it should have been, and
/// nothing downstream can tell those apart.
///
/// **A required source the Core holds no Watermark for** *is* reachable, and
/// `coverage_complete` does not exclude it: that predicate is `required ⊆
/// observed`, and being observed does not imply having declared a completeness
/// boundary — `Evaluator::evidence_watermarks` omits a source whose
/// `complete_through()` is `None`. `Evaluator::seal` builds Verdicts under no
/// gate at all, so a source that opens a fresh epoch after its queries settled,
/// and has not yet declared that epoch's boundary, arrives here absent.
///
/// **[`Ordering::Less`]** — the answer sits in an epoch the source has since
/// left — is reachable, and is the only non-[`Ordering::Equal`] arm the
/// `Evaluator` reaches. `FactLedger::declare` is idempotent by
/// `(binding, guard)`, so the floor is snapshotted into a `FactQuery` once and
/// never refreshed; `settles` pins the answer to that floor's epoch; nothing
/// pins that epoch to the one the ledger has reached by settlement time. The
/// arm clamps nothing on purpose. `sentinel_model` orders Watermarks as
/// `(epoch, sequence)` pairs, so an earlier epoch is already strictly below the
/// ceiling, and taking a minimum against a later epoch's sequence would compare
/// two numbers that count different things.
///
/// **[`Ordering::Greater`]** is unreachable from the `Evaluator` *today*, and
/// the reason is narrow enough to be worth stating rather than trusting.
/// `declare_queries` runs only inside `advance`'s
/// `gate.is_open() && gate.watermarks_cover_epochs()` branch, and
/// `watermarks_cover_epochs` is false whenever any required source lacks a
/// `complete_through`. So at declaration the floor names *every* required
/// source, `settles` pins every one of them to the floor's epoch, and evidence
/// epochs are monotone (`MonotonicityBreach::RegressedEpoch`) — which leaves
/// the answer's epoch at or below the evidence's, never above. The arm stays
/// for two reasons: `settle` owes totality to any crate-internal caller that ran
/// no such gate, and the arm goes live the moment the floor stops naming every
/// required source, which is one relaxation of `watermarks_cover_epochs` away.
/// **#17** is the candidate relaxation — it makes the gate per-Binding rather
/// than global — but it is not sufficient on its own, and the difference is
/// worth stating because the obvious version of this paragraph is wrong.
/// Stripping a required source's `complete_through` closes the gate *twice*:
/// it sets `watermarks_cover_epochs` false, and it also inserts
/// `GateReason::IncompleteDeliveryToWatermark`, because the gate then asks
/// `covers_watermark(source, epoch, 0)` and a source that delivered Events
/// cannot satisfy `distinct == 0 && highest == 0`. Deleting the coverage flag
/// from `advance`'s branch therefore leaves the arm unreachable; that is
/// verified by mutation in
/// `a_required_source_without_a_boundary_declares_no_query`.
///
/// So the arm is not defensive coding to be deleted as dead — it is guarded
/// twice over, and both guards have to go before it is live. That test says so,
/// and fails when they have.
///
/// # What #17 would actually have to break, stated positively
///
/// Knowing what did *not* work is only half of it. The hazard #17 introduces is
/// a **level mismatch**: it makes the gate per-Binding, while `required`
/// (`evaluator.rs:453`) and `evidence_watermarks` (`:439`) are both derived from
/// `specification.required_source_identities()` and stay specification-level. So
/// a per-Binding gate can open for one Binding while a specification-required
/// source `S` still lacks a `complete_through`; the floor omits `S`; this
/// function nevertheless iterates the specification-level `required` and reaches
/// `S`; and `evidence_watermarks.get(S)` returns `None`.
///
/// Which means the exit that goes live first after #17 is the **`?` degrade**,
/// not [`Ordering::Greater`]. `Greater` needs a further step — `S` acquiring a
/// `complete_through` between declaration and settlement, at an epoch below the
/// periphery's answer, with `settles` never constraining `S` because the floor
/// omitted it.
///
/// The decision #17 therefore owns is whether `required`, `evidence_watermarks`
/// and the floor become per-Binding alongside the gate. If they do, the arm
/// stays unreachable by the same argument as today. If they do not, the degrade
/// goes live first and this arm second.
/// Recording anything here would be fabrication in the exact field
/// `validate_decisive_event` uses to bound witnesses.
///
/// That same gate has a quieter consequence. `watermarks_cover_epochs` also
/// demands `complete_through == last_emitted`, which *closes* the epoch, and a
/// closed epoch cannot advance (`MonotonicityBreach::ReopenedClosedEpoch`). So
/// for a floor source the ceiling equals the floor exactly until the epoch
/// turns over, and [`Ordering::Equal`]'s minimum fires precisely when the
/// periphery answered past a boundary the source itself had declared closed.
fn clamp_to_evidence(
    settled_watermarks: &SettledWatermarks,
    evidence_watermarks: &EvidenceWatermarks,
    required: &BTreeSet<SourceIdentity>,
) -> Option<BTreeMap<SourceIdentity, WatermarkPoint>> {
    required
        .iter()
        .map(|source| {
            debug_assert!(
                settled_watermarks.contains_key(source),
                "a settled cold path must name every required source: \
                 FactResponse::settles refuses an answer that omits one, and \
                 this map is the union of the answers that passed it"
            );
            let (settled_epoch, settled_through) = *settled_watermarks.get(source)?;
            let (evidence_epoch, evidence_through) = *evidence_watermarks.get(source)?;
            let point = match settled_epoch.cmp(&evidence_epoch) {
                Ordering::Less => (settled_epoch, settled_through),
                Ordering::Equal => (settled_epoch, settled_through.min(evidence_through)),
                Ordering::Greater => return None,
            };
            Some((source.clone(), point))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{clamp_to_evidence, settle, settle_derived, BarrierState};
    use crate::evidence::EvidenceLedger;
    use crate::facts::{EvidenceWatermarks, SettledWatermarks};
    use crate::specification::{
        AttributeKey, EventSelector, PrecedenceSpecification, SpecificationId,
        SpecificationIdentity, SpecificationVersion,
    };
    use sentinel_model::{
        ActivationBarrier, ActivationWatermark, EventKind, FilterEpoch, MonitoredApplicationId,
        ObservationTime, SchemaVersion, SentinelClockId, SequenceEpoch, SequenceNumber, SourceId,
        SourceIdentity, TierSettlement, VerdictAuthority,
    };
    use std::collections::{BTreeMap, BTreeSet};

    use crate::facts::WatermarkPoint;

    /// The smallest Specification the settlement rule can be asked about.
    ///
    /// Nothing here is evaluated: `settle` reads only the barrier off it, and
    /// `BarrierState::of` reads only `activation_barrier()`.
    fn specification() -> PrecedenceSpecification {
        PrecedenceSpecification::new(
            SpecificationIdentity::new(
                SpecificationId::new("spec").expect("fixture"),
                SpecificationVersion::new("v1").expect("fixture"),
            ),
            AttributeKey::new("binding").expect("fixture"),
            EventSelector::new(
                EventKind::new("trigger").expect("constant kind"),
                BTreeMap::new(),
            ),
            EventSelector::new(
                EventKind::new("requirement").expect("constant kind"),
                BTreeMap::new(),
            ),
            BTreeSet::new(),
            SchemaVersion::new("v1").expect("constant schema version"),
        )
    }

    /// A [`BarrierState`] derived the only way one can be: from a Specification and its
    /// Evidence Coverage.
    ///
    /// The ledger is empty, so `Coverage::covered` is empty: a declared barrier finds no
    /// completeness signal for its source and is therefore **pending**, and an undeclared
    /// one is `NotDeclared` whatever the coverage says.
    fn barrier_state(barrier: Option<ActivationBarrier>) -> BarrierState {
        let specification = match barrier {
            Some(barrier) => specification().with_activation_barrier(barrier),
            None => specification(),
        };
        let ledger = EvidenceLedger::new();
        let coverage = ledger.coverage(BTreeSet::new());
        BarrierState::of(&specification, &coverage)
    }

    /// The state every pre-#19 call site implicitly had: no barrier declared.
    fn no_barrier() -> BarrierState {
        barrier_state(None)
    }

    /// An activation barrier over one source, which an empty ledger cannot clear.
    fn one_source_barrier() -> ActivationBarrier {
        ActivationBarrier::new(
            FilterEpoch::new(1),
            BTreeMap::from([(
                source("sagashop-order"),
                ActivationWatermark::new(
                    SequenceEpoch::new(0),
                    SequenceNumber::new(16),
                    ObservationTime::new(
                        SentinelClockId::new("sentinel").expect("constant clock id"),
                        0,
                    ),
                ),
            )]),
        )
        .expect("a barrier naming one source is constructible")
    }

    /// A declared barrier over one source, which the empty ledger cannot clear.
    fn pending_barrier() -> BarrierState {
        barrier_state(Some(one_source_barrier()))
    }

    fn source(service: &str) -> SourceIdentity {
        SourceIdentity::new(
            MonitoredApplicationId::new(service).expect("service name is not empty"),
            SourceId::new("sagashop").expect("constant source id"),
        )
    }

    fn point(epoch: u64, sequence: u64) -> WatermarkPoint {
        (SequenceEpoch::new(epoch), SequenceNumber::new(sequence))
    }

    /// Every settling path runs the same three gates; only the maps vary.
    fn settled(
        answers: &SettledWatermarks,
        evidence: &EvidenceWatermarks,
        required: &BTreeSet<SourceIdentity>,
    ) -> super::Settlement {
        settle_derived(no_barrier(), true, true, true, answers, evidence, required)
    }

    /// The settling answers' union.
    fn answers<const N: usize>(
        entries: [(SourceIdentity, WatermarkPoint); N],
    ) -> SettledWatermarks {
        SettledWatermarks::new(BTreeMap::from(entries))
    }

    /// The Core's own Watermark — the ceiling.
    fn evidence<const N: usize>(
        entries: [(SourceIdentity, WatermarkPoint); N],
    ) -> EvidenceWatermarks {
        EvidenceWatermarks::new(BTreeMap::from(entries))
    }

    /// Within one epoch the recorded coordinate is the evidence, not the answer.
    ///
    /// This pins the clamp but — deliberately noted, because it is the trap this
    /// whole module of tests exists for — it **cannot** detect the two maps being
    /// passed in the wrong order. The `Equal` arm's body is
    /// `settled_through.min(evidence_through)`, and `min` is commutative, so a
    /// transposition is a genuine no-op here. The two epoch tests below are the
    /// ones that discriminate.
    #[test]
    fn the_clamp_records_the_evidence_ceiling_within_an_epoch() {
        let order = source("sagashop-order");
        let answers = answers([(order.clone(), point(0, 20))]);
        let evidence = evidence([(order.clone(), point(0, 16))]);
        let required = BTreeSet::from([order.clone()]);

        let settlement = settled(&answers, &evidence, &required);

        assert_eq!(settlement.cold_path(), TierSettlement::Settled);
        assert_eq!(settlement.authority(), VerdictAuthority::Authoritative);
        assert_eq!(
            settlement.cold_settlement_watermarks().get(&order),
            Some(&point(0, 16)),
            "the answer ran ahead of the Core's evidence; the record must not"
        );
    }

    /// The ordering the `Less` arm rests on, pinned rather than assumed.
    ///
    /// `WatermarkPoint` is a tuple, so its `Ord` is lexicographic *provided*
    /// each field's own `Ord` is the natural one — and both `SequenceEpoch` and
    /// `SequenceNumber` derive it over a single `u64`. If either ever gains a
    /// hand-written `Ord`, or the tuple becomes a struct with the fields
    /// reordered, the `Less` arm silently starts recording a coordinate the
    /// answer never covered. `sentinel_model`'s `validate_settlement_reached`
    /// makes the same lexicographic assumption over its own `(u64, u64)`
    /// coordinate, so the two must not drift apart.
    ///
    /// This is the assumption that licenses recording an earlier-epoch answer
    /// unclamped, in the exact field `validate_decisive_event` uses to bound
    /// witnesses. It is cheap to pin and expensive to be wrong about.
    #[test]
    fn a_watermark_point_orders_lexicographically_not_componentwise() {
        assert!(
            point(0, 500) < point(1, 10),
            "a later epoch outranks any sequence in an earlier one"
        );
        assert!(
            point(0, 16) < point(0, 20),
            "within an epoch, sequence orders"
        );
        assert!(
            point(1, 0) > point(0, u64::MAX),
            "an epoch bump outranks even an exhausted predecessor"
        );
    }

    /// An answer from an epoch the source has since left is recorded unclamped.
    ///
    /// `WatermarkPoint` is `(SequenceEpoch, SequenceNumber)`, so its ordering is
    /// lexicographic and `(0, 500)` is already strictly below `(1, 10)`. There is
    /// nothing to clamp, and clamping would *lower* a coordinate the answer
    /// genuinely covered.
    ///
    /// **This test discriminates the argument order.** Transposed, the comparison
    /// reads `(1, 10).cmp(&(0, 500))` — `Greater` — and the whole tier degrades to
    /// `Pending`/`Provisional` instead of settling.
    #[test]
    fn an_answer_from_an_earlier_epoch_is_recorded_unclamped() {
        let order = source("sagashop-order");
        let answers = answers([(order.clone(), point(0, 500))]);
        let evidence = evidence([(order.clone(), point(1, 10))]);
        let required = BTreeSet::from([order.clone()]);

        let settlement = settled(&answers, &evidence, &required);

        assert_eq!(settlement.cold_path(), TierSettlement::Settled);
        assert_eq!(settlement.authority(), VerdictAuthority::Authoritative);
        assert_eq!(
            settlement.cold_settlement_watermarks().get(&order),
            Some(&point(0, 500))
        );
    }

    /// An answer from a *later* epoch degrades rather than fabricating coverage.
    ///
    /// A [`SequenceEpoch`] bump is a sequence reset, so `(1, 0)` implies nothing
    /// about coverage through `(0, 16)`. Clamping it to the evidence would assert
    /// coverage of a sequence space the answer never touched — fabrication by the
    /// Core in the field `validate_decisive_event` uses to bound witnesses.
    ///
    /// **This test discriminates the argument order.** Transposed, the comparison
    /// reads `(0, 16).cmp(&(1, 0))` — `Less` — and it settles at `(0, 16)` instead
    /// of degrading.
    #[test]
    fn an_answer_from_a_later_epoch_degrades_rather_than_fabricating() {
        let order = source("sagashop-order");
        let answers = answers([(order.clone(), point(1, 0))]);
        let evidence = evidence([(order.clone(), point(0, 16))]);
        let required = BTreeSet::from([order.clone()]);

        let settlement = settled(&answers, &evidence, &required);

        assert_eq!(settlement.cold_path(), TierSettlement::Pending);
        assert_eq!(settlement.authority(), VerdictAuthority::Provisional);
        assert!(
            settlement.cold_settlement_watermarks().is_empty(),
            "a degraded tier records no coordinate at all"
        );
    }

    /// A required source the Core holds no Watermark for degrades the tier.
    ///
    /// Reachable: `coverage_complete` is `required.is_subset(observed_sources)`,
    /// and being *observed* does not imply holding a completeness boundary —
    /// `Evaluator::watermark_floor` skips exactly this case with a `continue`.
    #[test]
    fn a_required_source_absent_from_the_evidence_degrades() {
        let order = source("sagashop-order");
        let payment = source("sagashop-payment");
        let answers = answers([
            (order.clone(), point(0, 16)),
            (payment.clone(), point(0, 9)),
        ]);
        let evidence = evidence([(order.clone(), point(0, 16))]);
        let required = BTreeSet::from([order, payment]);

        let settlement = settled(&answers, &evidence, &required);

        assert_eq!(settlement.cold_path(), TierSettlement::Pending);
        assert_eq!(settlement.authority(), VerdictAuthority::Provisional);
        assert!(settlement.cold_settlement_watermarks().is_empty());
    }

    /// A required source no settling answer named is a caller-contract break.
    ///
    /// Not an evidence condition, so it is not degraded quietly. `settles`
    /// refuses an answer omitting a required source and this map is the union of
    /// the answers that passed it, so reaching here means a caller ran no gate.
    /// The `debug_assert!` fires in test and debug builds; the `?` beneath it is
    /// the release fallback, which degrades rather than panicking in production.
    #[test]
    #[should_panic(expected = "must name every required source")]
    fn a_required_source_absent_from_the_answers_trips_the_caller_contract() {
        let order = source("sagashop-order");
        let payment = source("sagashop-payment");
        let answers = answers([(order.clone(), point(0, 16))]);
        let evidence = evidence([
            (order.clone(), point(0, 16)),
            (payment.clone(), point(0, 9)),
        ]);
        let required = BTreeSet::from([order, payment]);

        let _ = settled(&answers, &evidence, &required);
    }

    /// The record is restricted to `required`; a bystander source never appears.
    #[test]
    fn a_source_outside_the_required_set_is_not_recorded() {
        let order = source("sagashop-order");
        let bystander = source("sagashop-search");
        let answers = answers([
            (order.clone(), point(0, 16)),
            (bystander.clone(), point(0, 4)),
        ]);
        let evidence = evidence([
            (order.clone(), point(0, 16)),
            (bystander.clone(), point(0, 4)),
        ]);
        let required = BTreeSet::from([order.clone()]);

        let settlement = settled(&answers, &evidence, &required);

        assert_eq!(settlement.cold_settlement_watermarks().len(), 1);
        assert!(!settlement
            .cold_settlement_watermarks()
            .contains_key(&bystander));
    }

    /// The clamp is only consulted once the tier has actually settled.
    #[test]
    fn an_unsettled_tier_records_no_coordinate() {
        let order = source("sagashop-order");
        let answers = answers([(order.clone(), point(0, 16))]);
        let evidence = evidence([(order.clone(), point(0, 16))]);
        let required = BTreeSet::from([order]);

        for (complete, guards, every) in [(false, true, true), (true, true, false)] {
            let settlement = settle_derived(
                no_barrier(),
                complete,
                guards,
                every,
                &answers,
                &evidence,
                &required,
            );
            assert_eq!(settlement.cold_path(), TierSettlement::Pending);
            assert!(settlement.cold_settlement_watermarks().is_empty());
        }
    }

    /// A Specification declaring no cold guards needs no cold evidence.
    #[test]
    fn a_guard_free_specification_is_not_required_rather_than_pending() {
        let required = BTreeSet::new();

        let settlement = settle_derived(
            no_barrier(),
            true,
            false,
            false,
            &SettledWatermarks::default(),
            &EvidenceWatermarks::default(),
            &required,
        );

        assert_eq!(settlement.cold_path(), TierSettlement::NotRequired);
        assert_eq!(settlement.authority(), VerdictAuthority::Authoritative);
    }

    /// A pending activation barrier caps authority even when nothing else is unsettled.
    ///
    /// Coverage is complete, guards are declared, every query settled, and the clamp
    /// produced a coordinate — and the barrier preserves that coordinate. Compare
    /// [`the_clamp_records_the_evidence_ceiling_within_an_epoch`], which is the identical
    /// call with no barrier declared and settles `Authoritative` at `(0, 16)`.
    #[test]
    fn a_pending_barrier_caps_authority_and_preserves_the_cold_coordinate() {
        let order = source("sagashop-order");
        let answers = answers([(order.clone(), point(0, 20))]);
        let evidence = evidence([(order.clone(), point(0, 16))]);
        let required = BTreeSet::from([order]);

        let settlement = settle_derived(
            pending_barrier(),
            true,
            true,
            true,
            &answers,
            &evidence,
            &required,
        );

        assert_eq!(settlement.authority(), VerdictAuthority::Provisional);
        assert_eq!(settlement.cold_path(), TierSettlement::Settled);
        assert_eq!(
            settlement
                .cold_settlement_watermarks()
                .get(&source("sagashop-order")),
            Some(&point(0, 16)),
            "the barrier caps authority; it does not discard a coordinate the cold path reached"
        );
    }

    /// The cap is the *only* lever a pending barrier pulls: a guard-free Specification at
    /// complete coverage is `NOT_REQUIRED`/`Authoritative` without it and
    /// `NOT_REQUIRED`/`Provisional` with it.
    #[test]
    fn the_cap_is_the_only_difference_a_pending_barrier_makes() {
        let required = BTreeSet::new();
        let call = |barrier: BarrierState| {
            settle_derived(
                barrier,
                true,
                false,
                false,
                &SettledWatermarks::default(),
                &EvidenceWatermarks::default(),
                &required,
            )
        };

        let without = call(no_barrier());
        assert_eq!(without.cold_path(), TierSettlement::NotRequired);
        assert_eq!(without.authority(), VerdictAuthority::Authoritative);

        let with = call(pending_barrier());
        assert_eq!(with.cold_path(), TierSettlement::NotRequired);
        assert_eq!(with.authority(), VerdictAuthority::Provisional);
    }

    /// The public entry point derives every Specification-shaped fact from one value.
    ///
    /// The regression that guards the substitution fix. Two calls, identical in every
    /// argument the caller supplies; the *only* difference is which Specification is
    /// passed. There is no barrier argument to set independently of it, so the laundering
    /// that motivated this shape — real Specification, decoy barrier — has no parameter to
    /// travel through.
    ///
    /// It also pins that `settle` really does read `coverage_complete`, `guards_declared`
    /// and `required` off the Specification and Coverage rather than ignoring them: the
    /// barrier-free call settles `NOT_REQUIRED`/`Authoritative`, which is only reachable
    /// when `coverage_complete` is true and `guards_declared` is false.
    #[test]
    fn the_public_entry_point_derives_every_specification_fact_from_one_value() {
        let ledger = EvidenceLedger::new();
        let coverage = ledger.coverage(BTreeSet::new());
        let call = |specification: &PrecedenceSpecification| {
            settle(
                specification,
                &coverage,
                false,
                &SettledWatermarks::default(),
                &EvidenceWatermarks::default(),
            )
        };

        let without = call(&specification());
        assert_eq!(without.cold_path(), TierSettlement::NotRequired);
        assert_eq!(without.authority(), VerdictAuthority::Authoritative);
        assert!(!without.barrier().is_declared());

        let warming = specification().with_activation_barrier(one_source_barrier());
        let with = call(&warming);
        assert_eq!(with.cold_path(), TierSettlement::NotRequired);
        assert_eq!(with.authority(), VerdictAuthority::Provisional);
        assert!(with.barrier().is_pending());
    }

    /// `Settlement` carries the state `settle` used, so the Verdict cannot disclose one
    /// barrier state while the authority was capped under another.
    #[test]
    fn the_settlement_carries_the_barrier_state_it_was_computed_under() {
        let ledger = EvidenceLedger::new();
        let coverage = ledger.coverage(BTreeSet::new());
        let warming = specification().with_activation_barrier(one_source_barrier());

        let settlement = settle(
            &warming,
            &coverage,
            false,
            &SettledWatermarks::default(),
            &EvidenceWatermarks::default(),
        );

        assert_eq!(
            settlement.barrier(),
            &BarrierState::of(&warming, &coverage),
            "the carried state is the derived one, not a second computation"
        );
        assert!(
            settlement.barrier().pending().is_some(),
            "crate::verdict reads the pending detail off exactly this value"
        );
    }

    /// The three barrier states are distinct, and "declares none" is not "cleared".
    ///
    /// Conflating them would let a consumer read a Specification that never declared a
    /// barrier as one whose rollout completed.
    #[test]
    fn not_declared_is_not_the_same_as_cleared() {
        let none = no_barrier();
        assert!(!none.is_declared());
        assert!(!none.is_cleared());
        assert!(!none.is_pending());
        assert_eq!(none.cleared(), None);
        assert_eq!(none.pending(), None);

        let pending = pending_barrier();
        assert!(pending.is_declared());
        assert!(!pending.is_cleared());
        assert!(pending.is_pending());
        assert_eq!(pending.cleared(), None);
        assert!(
            pending.pending().is_some(),
            "a pending barrier names what is holding it"
        );
    }

    /// Direct cover for the helper's contract, independent of `settle`'s gates.
    ///
    /// Only the two *evidence* conditions are exercised here. The third exit —
    /// a required source missing from the answers — is a contract break rather
    /// than an evidence condition and is pinned by its own `should_panic` test.
    #[test]
    fn clamp_returns_none_for_every_unsound_evidence_case() {
        let order = source("sagashop-order");
        let required = BTreeSet::from([order.clone()]);
        let present_settled = answers([(order.clone(), point(0, 16))]);
        let present_evidence = evidence([(order.clone(), point(0, 16))]);
        let empty = EvidenceWatermarks::default();

        assert!(
            clamp_to_evidence(&present_settled, &empty, &required).is_none(),
            "no evidence Watermark means no ceiling to clamp against"
        );
        assert!(
            clamp_to_evidence(
                &answers([(order, point(1, 0))]),
                &present_evidence,
                &required
            )
            .is_none(),
            "a later epoch cannot be clamped without fabricating coverage"
        );
    }

    /// Names every producer the `compile_fail` doctests on [`super::Settlement`] list.
    ///
    /// Stable rustdoc does not check a `compile_fail` block's error code, so a block naming
    /// a renamed or removed item fails with `E0425` and still passes. This test makes that
    /// rename or removal break the `--lib` build instead, and whoever made it must carry the
    /// doctest list forward. It guards **names only**: privacy is guarded by the doctests
    /// themselves, and the seal by `Settlement` having no producer outside this crate (#304).
    #[test]
    fn producers_named_by_the_sealed_doctests_still_exist() {
        let _ = super::settle;
        let _ = super::settle_within;
        let _ = super::settle_taint;
        let _ = super::settle_taint_within;
        let _ = super::settle_absence;
        let _ = super::settle_absence_within;
        let _ = super::settle_aggregate;
        let _ = super::settle_aggregate_within;
        let _ = super::settle_existence;
        let _ = super::settle_existence_within;
    }
}
