//! The evidence ledger: what each source declared, and what the Core received.
//!
//! This is the accumulator that replaces the Adapter batch the #10 Core took by
//! value. It holds two *different kinds of claim*, and keeping them apart is
//! the whole point of the module:
//!
//! - **What the source said.** [`SourceEvidence`]: tier, capabilities, the
//!   completeness signal, and the gaps the Adapter computed over the sequences
//!   *it emitted*. Contiguity (`gaps.is_empty()`) is a claim about production.
//! - **What the Core received.** Per `(source, epoch)` receipt counters,
//!   derived from `Frontier::delivered()` — after duplicate suppression, and
//!   deliberately **not** from `Frontier::admitted()`, because a held Event's
//!   source would silently drop out of the observed set and move
//!   `Coverage::is_complete`, which moves both `authority` and `cold_path`.
//!
//! # The evidence monotonicity contract (issue #12, §2.4)
//!
//! `Step::Settled` is a promise the Core cannot keep unless evidence is
//! monotone: the evidence gate is global, so a later source signal or a later
//! Event could otherwise flip an already-emitted Verdict, and **no Amendment
//! record exists anywhere in `proto/` or `model/`** to retract it with
//! (ADR-0021's open O-11). The contract has three rules; (a) and (b) are
//! checked here and error, (c) is a gate condition and downgrades.
//!
//! **(a) Signal monotonicity, over the whole record.** Per `(source, epoch)`:
//! `epoch`, `last_emitted` and `complete_through` never regress — *and* a
//! closed epoch stays closed — *and* `tier`, `capabilities` and `gaps` are
//! never restated. All five of [`SourceEvidence`]'s fields are gate or
//! assessment inputs, so covering only the completeness bounds would leave a
//! restated `tier` free to flip an already-emitted `Violate` to `Inconclusive`
//! through the anchor-candidate filter, and a restated `gaps` free to do the
//! same through G2. The escape hatch for every clause is identical: a new
//! [`SequenceEpoch`].
//! Once `complete_through == last_emitted` for a `(source, epoch)`, neither
//! value may advance within that epoch; a source with more to say opens a new
//! [`SequenceEpoch`]. The second clause is not an extra concept: rule (b)
//! already *defines* closure as `complete_through == last_emitted`, and tests
//! the boundary against the current signal, so without it a source could
//! relicense the very Events (b) rejects simply by advancing the boundary.
//! What that would allow: a Binding settles `Inconclusive` at a closed boundary
//! of 4, the source then declares 6, slots 5 and 6 arrive carrying the matching
//! requirement Event, and the Binding re-emits as `Satisfy` — an outcome change
//! after emission, unretractable.
//!
//! **(b) No delivery above a closed boundary.** Once an epoch is closed, no
//! Event from that source in that epoch with `sequence > complete_through`.
//!
//! **(b′) Ordering rule, permanent and not incidental.** Rule (b) is checked
//! **after** `Frontier::step`'s delivery-conflict classification, so a
//! conflicting delivery is always reported as [`crate::CoreError::Delivery`]
//! and never as [`crate::CoreError::NonMonotonicEvidence`]. Today the
//! Events-then-Sources driver order makes that hold anyway — no boundary is
//! known during the Event phase — but that is a coincidence of one driver, and
//! a future Sources-first periphery must not silently violate it.
//!
//! **(c) The Core's receipts must cover the boundary.** For each *required*
//! source whose `gaps.is_empty()`, the Core's delivered slot set for that epoch
//! must be exactly `{1..=complete_through}`. This is [`EvidenceLedger::covers_watermark`],
//! and it is the gate disjunct `IncompleteDeliveryToWatermark`. It is
//! **cardinality**, not boundary equality: "max delivered slot ==
//! complete_through" proves only that the Core reached the *top* of the range,
//! and `gaps` cannot cover the interior because `gaps` lives in
//! source-emission space, not Core-receipt space. A source can emit slots
//! 1..4 truthfully while the Core holds `{1, 3, 4}`, and a boundary-only check
//! would open the gate over a missing interior anchor Event.
//!
//! Together (a), (b) and (c) give the lemma `seal()` rests on: the gate opens
//! only when every sequence in `[1, complete_through]` has been delivered *to
//! the Core*, so a post-settlement Event at or below the boundary is
//! necessarily a duplicate or a slot conflict — both already handled by
//! `Frontier::step` — and everything above it is rule (b). Settlement is
//! monotone, at the top of the range and in the middle, which is what makes
//! `seal()` a theorem rather than "trust the caller".
//!
//! # Evidence Coverage is the conjunction of the two kinds (issue #158)
//!
//! The two kinds of claim above are populated by two different methods —
//! [`EvidenceLedger::record_source`] and [`EvidenceLedger::record_delivery`] —
//! and a source can supply either one without the other. [`Coverage`] is the
//! only place they are joined, and it joins them by **carrying the record**: a
//! source enters [`Coverage::covered`] together with the [`SourceEvidence`] it
//! declared, or it does not enter at all.
//!
//! Before #158 that join was three separate expressions and they had already
//! drifted: the evidence gate computed the conjunction, while
//! `Evaluator::build_verdicts` and `verdict::Coverage` read only the delivery
//! half, and `verdict::assurance_envelope` then asserted the conjunction with
//! `.expect("covered sources supplied evidence")` over a set that did not
//! establish it. An ordinary driver — deliver Events, forget one
//! `record_source`, `seal()` — panicked. ADR-0031 §1: the invariant lives in
//! the type now, and the panic is unconstructible rather than unlikely.
//!
//! # The evidence gate
//!
//! [`EvidenceGate`] is those two kinds of claim plus Frontier G4 (held Events
//! and unresolved causal edges). It evaluates over required identities, the
//! ledger, and the Frontier — the same required set
//! [`EvidenceLedger::coverage`] already takes.
//! [`EvidenceGate::watermarks_cover_epochs`] is Coverage completeness, a second
//! conjunct on the type, not a sixth [`GateReason`].

#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use sentinel_model::{Event, SourceEvidence, SourceIdentity, SourceTier};

use crate::frontier::Frontier;

/// Which clause of the evidence contract an input broke.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum MonotonicityBreach {
    /// (a) A source's sequence epoch went backwards.
    RegressedEpoch,
    /// (a) `last_emitted` went backwards within one epoch.
    RegressedLastEmitted,
    /// (a) `complete_through` went backwards within one epoch.
    RegressedCompleteThrough,
    /// (a) A closed epoch advanced instead of a new epoch being opened.
    ReopenedClosedEpoch,
    /// (a) The source was restated at a different assurance tier.
    RedeclaredTier,
    /// (a) The source was restated with different verified capabilities.
    RedeclaredCapabilities,
    /// (a) The source was restated with a different set of sequence gaps.
    RedeclaredGaps,
    /// (a) The acknowledged Filter epoch went backwards within one epoch.
    RegressedFilterEpoch,
    /// (b) An Event arrived above a closed boundary.
    DeliveryAboveClosedBoundary,
}

impl MonotonicityBreach {
    /// Returns a stable, human-readable description.
    #[must_use]
    pub const fn detail(self) -> &'static str {
        match self {
            Self::RegressedEpoch => "the source's sequence epoch regressed",
            Self::RegressedLastEmitted => "last_emitted regressed within one epoch",
            Self::RegressedCompleteThrough => "complete_through regressed within one epoch",
            Self::ReopenedClosedEpoch => {
                "a closed epoch advanced; a source with more to say opens a new SequenceEpoch"
            }
            Self::RedeclaredTier => "the source was restated at a different tier within one epoch",
            Self::RedeclaredCapabilities => {
                "the source was restated with different capabilities within one epoch"
            }
            Self::RedeclaredGaps => "the source was restated with different gaps within one epoch",
            Self::RegressedFilterEpoch => {
                "the acknowledged Filter epoch regressed within one epoch"
            }
            Self::DeliveryAboveClosedBoundary => {
                "an Event arrived above a boundary the source already closed"
            }
        }
    }
}

/// Evidence that contradicts what the same source already declared.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct NonMonotonicEvidence {
    source: SourceIdentity,
    epoch: u64,
    breach: MonotonicityBreach,
}

impl NonMonotonicEvidence {
    /// Returns the source whose evidence regressed.
    #[must_use]
    pub const fn source(&self) -> &SourceIdentity {
        &self.source
    }

    /// Returns the sequence epoch the breach falls in.
    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Returns which clause of the contract was broken.
    #[must_use]
    pub const fn breach(&self) -> MonotonicityBreach {
        self.breach
    }
}

impl Display for NonMonotonicEvidence {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}/{} epoch {}: {}",
            self.source.monitored_application().as_str(),
            self.source.source().as_str(),
            self.epoch,
            self.breach.detail()
        )
    }
}

impl Error for NonMonotonicEvidence {}

/// What one source declared about one epoch, most recently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Declaration {
    last_emitted: u64,
    complete_through: u64,
    closed: bool,
}

/// What the Core received from one source in one epoch.
///
/// `distinct` counts delivery slots, not deliveries: `Frontier` suppresses a
/// duplicate and rejects every other same-slot redelivery, so the slots counted
/// here are genuinely distinct values.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Receipts {
    distinct: u64,
    highest: u64,
}

/// Everything the Core knows about its sources, accumulated input by input.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EvidenceLedger {
    sources: BTreeMap<SourceIdentity, SourceEvidence>,
    declarations: BTreeMap<(SourceIdentity, u64), Declaration>,
    receipts: BTreeMap<(SourceIdentity, u64), Receipts>,
    observed: BTreeSet<SourceIdentity>,
}

impl EvidenceLedger {
    /// Creates an empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one source's latest evidence, enforcing contract clause (a).
    ///
    /// # Errors
    ///
    /// Returns [`NonMonotonicEvidence`] when the signal regresses, when it
    /// advances an epoch the same source already closed, when it restates the
    /// source's tier, capabilities or gaps within an epoch, or when it
    /// acknowledges a Filter epoch below one it already acknowledged within an
    /// epoch.
    pub fn record_source(&mut self, evidence: SourceEvidence) -> Result<(), NonMonotonicEvidence> {
        let source = evidence.source().clone();
        let signal = evidence.completeness();
        let epoch = signal.epoch().value();
        let last_emitted = signal
            .last_emitted()
            .map_or(0, sentinel_model::SequenceNumber::value);
        let complete_through = signal
            .complete_through()
            .map_or(0, sentinel_model::SequenceNumber::value);

        let breach = |breach| NonMonotonicEvidence {
            source: source.clone(),
            epoch,
            breach,
        };

        if let Some(highest) = self.highest_declared_epoch(&source) {
            if epoch < highest {
                return Err(breach(MonotonicityBreach::RegressedEpoch));
            }
        }
        if let Some(previous) = self.declarations.get(&(source.clone(), epoch)).copied() {
            if last_emitted < previous.last_emitted {
                return Err(breach(MonotonicityBreach::RegressedLastEmitted));
            }
            if complete_through < previous.complete_through {
                return Err(breach(MonotonicityBreach::RegressedCompleteThrough));
            }
            if previous.closed
                && (last_emitted > previous.last_emitted
                    || complete_through > previous.complete_through)
            {
                return Err(breach(MonotonicityBreach::ReopenedClosedEpoch));
            }
        }

        // The non-completeness half of the record is monotone too, and for the
        // same reason. `tier` feeds the anchor-candidate filter and both
        // `source_tier_gap` filters; `gaps` feeds `is_contiguous()` and hence
        // G2; `capabilities` is written straight into the envelope. All three
        // can move an already-emitted Verdict's outcome or bytes, so a bare
        // overwrite would reopen exactly the hole the closed-epoch clause was
        // added to close — a second `Step::Settled` for one Binding in one
        // epoch, with a different conclusion and no Amendment record to retract
        // the first. Issue #12's own tier fix is what promoted `tier` from a
        // Specification constant to a per-input evidence field, so this clause
        // is the price of that fix. The escape hatch is the same as clause
        // (a)'s: a source whose deployment facts changed opens a new
        // `SequenceEpoch`.
        //
        // `acknowledged_filter_epoch` joined that list in #19, and it is worth
        // saying why it was absent rather than pretending it was an oversight.
        // Until the activation barrier existed, nothing turned a source's
        // acknowledged Filter epoch into authority: the field was recorded in
        // the envelope and read by no rule, so a source restating it moved no
        // Verdict. `crate::settlement::BarrierState` makes it load bearing, and
        // makes the omission reachable in the precise shape `crate`'s
        // limitation 8 warns about — the contract has to cover **every** field
        // of `SourceEvidence`, not only the completeness bounds.
        //
        // Executed before the check existed: a required source re-declaring at
        // a lower acknowledged epoch flipped its barrier cleared → pending,
        // `Evaluator::advance` re-emitted because the encoded Verdict changed,
        // and the result was `AT FILTER EPOCH 7: [Authoritative]` followed by
        // `AFTER FILTER-EPOCH DROP re-emitted: [Provisional]`. A driver wiring
        // the Core to `sentinel-store` then takes
        // `StoreError::Conflict(VerdictAuthorityRegressed)` on a Verdict the
        // Core considers correct — a periphery-visible failure caused by a Core
        // emission, which is what the monotonicity contract exists to prevent.
        //
        // Only the **backwards** direction is refused. A source acknowledging a
        // *newer* Filter epoch is ordinary forward rollout progress and is how
        // a barrier clears at all; the resulting `Provisional → Authoritative`
        // move is the authority upgrade limitation 8 explicitly blesses and the
        // store explicitly permits.
        if let Some(previous) = self.sources.get(&source) {
            if previous.completeness().epoch().value() == epoch {
                if previous.tier() != evidence.tier() {
                    return Err(breach(MonotonicityBreach::RedeclaredTier));
                }
                if previous.capabilities() != evidence.capabilities() {
                    return Err(breach(MonotonicityBreach::RedeclaredCapabilities));
                }
                if previous.gaps() != evidence.gaps() {
                    return Err(breach(MonotonicityBreach::RedeclaredGaps));
                }
                if signal.filter_epoch() < previous.completeness().filter_epoch() {
                    return Err(breach(MonotonicityBreach::RegressedFilterEpoch));
                }
            }
        }

        self.declarations.insert(
            (source.clone(), epoch),
            Declaration {
                last_emitted,
                complete_through,
                closed: complete_through == last_emitted,
            },
        );
        self.sources.insert(source, evidence);
        Ok(())
    }

    /// Records one delivered Event, enforcing contract clause (b).
    ///
    /// `newly_delivered` is false for a delivery `Frontier` suppressed as a
    /// duplicate, which must not move the distinct-slot counter clause (c)
    /// reads.
    ///
    /// # Errors
    ///
    /// Returns [`NonMonotonicEvidence`] when the Event sits above a boundary
    /// the source already closed. Per rule (b′) the caller must have run
    /// `Frontier::step`'s conflict classification **first**, so that a
    /// conflicting delivery is reported as a conflict rather than as a
    /// contract breach.
    pub fn record_delivery(
        &mut self,
        event: &Event,
        newly_delivered: bool,
    ) -> Result<(), NonMonotonicEvidence> {
        let slot = event.delivery();
        let source = slot.source().clone();
        let epoch = slot.epoch().value();
        let sequence = slot.sequence().value();

        if let Some(declaration) = self.declarations.get(&(source.clone(), epoch)) {
            if declaration.closed && sequence > declaration.complete_through {
                return Err(NonMonotonicEvidence {
                    source,
                    epoch,
                    breach: MonotonicityBreach::DeliveryAboveClosedBoundary,
                });
            }
        }

        self.observed.insert(source.clone());
        if newly_delivered {
            let receipts = self.receipts.entry((source, epoch)).or_default();
            receipts.distinct += 1;
            receipts.highest = receipts.highest.max(sequence);
        }
        Ok(())
    }

    /// Returns the highest sequence epoch this source has ever declared.
    fn highest_declared_epoch(&self, source: &SourceIdentity) -> Option<u64> {
        self.declarations
            .keys()
            .filter(|(declared, _)| declared == source)
            .map(|(_, epoch)| *epoch)
            .max()
    }

    /// Returns evidence for one source, when it supplied any.
    #[must_use]
    pub fn source(&self, source: &SourceIdentity) -> Option<&SourceEvidence> {
        self.sources.get(source)
    }

    /// Returns every source that supplied at least one delivered Event.
    ///
    /// Derived from `Frontier::delivered()`, never from `admitted()` (D3).
    ///
    /// This is **one half** of Evidence Coverage and never a substitute for it:
    /// a source here may have declared no [`SourceEvidence`] at all. Anything
    /// that needs a source's record as well as its Events reads
    /// [`EvidenceLedger::coverage`], which is the only place the two halves are
    /// joined.
    #[must_use]
    pub const fn observed_sources(&self) -> &BTreeSet<SourceIdentity> {
        &self.observed
    }

    /// Returns Evidence Coverage over one Specification's required sources.
    ///
    /// The only place a [`Coverage`] comes into existence, and therefore the
    /// only place the conjunction is written down. The evidence gate, the
    /// settlement predicate and the Assurance Envelope all read this one
    /// population rather than each recomputing an expression that ought to
    /// agree with the other two — which is what #158 found they had stopped
    /// doing.
    #[must_use]
    pub fn coverage(&self, required: BTreeSet<SourceIdentity>) -> Coverage<'_> {
        let mut covered = BTreeMap::new();
        let mut undeclared = BTreeSet::new();
        for source in &required {
            let delivered = self.observed.contains(source);
            match self.sources.get(source) {
                Some(evidence) if delivered => {
                    covered.insert(source.clone(), evidence);
                }
                // Events arrived and the source never declared itself. It is
                // not covered, and naming it is the diagnosis a driver needs.
                None if delivered => {
                    undeclared.insert(source.clone());
                }
                // Declared but silent, or neither: uncovered, and already
                // reported as such by the gate's G1 disjunct.
                _ => {}
            }
        }
        Coverage {
            required,
            covered,
            undeclared,
        }
    }

    /// Returns the tier the **evidence** places a source at (ADR-0022).
    ///
    /// This is the source of truth for tier, not
    /// [`crate::specification::PrecedenceSpecification::minimum_tier_of`]: the
    /// Specification declares the minimum tier a trustworthy conclusion needs;
    /// the deployment declares where the source actually sits.
    #[must_use]
    pub fn tier_of(&self, source: &SourceIdentity) -> Option<SourceTier> {
        self.sources.get(source).map(SourceEvidence::tier)
    }

    /// Returns whether the Core holds every slot in `[1, complete_through]`.
    ///
    /// Contract clause (c). Cardinality equality over distinct values in
    /// `[1, complete_through]` **is** set equality by pigeonhole, so this is an
    /// `O(1)` counter comparison and not a scan — but both halves are needed.
    /// Requiring only `highest == complete_through` leaves an interior hole
    /// open; requiring only `distinct == complete_through` leaves a slot above
    /// the boundary standing in for a missing slot below it.
    #[must_use]
    pub fn covers_watermark(&self, source: &SourceIdentity, epoch: u64, boundary: u64) -> bool {
        let receipts = self
            .receipts
            .get(&(source.clone(), epoch))
            .copied()
            .unwrap_or_default();
        receipts.distinct == boundary && receipts.highest == boundary
    }

    /// Returns how many distinct slots the Core holds for one `(source, epoch)`.
    ///
    /// Exposed so a test can state the cardinality-versus-boundary distinction
    /// directly rather than inferring it from a gate outcome.
    #[must_use]
    pub fn delivered_slot_count(&self, source: &SourceIdentity, epoch: u64) -> u64 {
        self.receipts
            .get(&(source.clone(), epoch))
            .map_or(0, |receipts| receipts.distinct)
    }

    /// Returns the highest slot the Core holds for one `(source, epoch)`.
    #[must_use]
    pub fn highest_delivered_slot(&self, source: &SourceIdentity, epoch: u64) -> u64 {
        self.receipts
            .get(&(source.clone(), epoch))
            .map_or(0, |receipts| receipts.highest)
    }

    /// Returns whether a source's signal contradicts what it actually delivered.
    ///
    /// This is gate disjunct G3, and it is deliberately **not** a restatement
    /// of G2. A gap is the source truthfully reporting its own discontinuity; a
    /// contradiction is the source's completeness claim disagreeing with the
    /// Core's receipts, which no truthful source produces. Under the
    /// monotonicity contract the disjunct is permanent once tripped rather than
    /// a transient artefact of interleaving.
    #[must_use]
    pub fn contradicts_delivery(&self, source: &SourceIdentity) -> bool {
        let Some(evidence) = self.sources.get(source) else {
            return false;
        };
        let signal = evidence.completeness();
        let epoch = signal.epoch().value();
        let last_emitted = signal
            .last_emitted()
            .map_or(0, sentinel_model::SequenceNumber::value);
        for ((receipt_source, receipt_epoch), receipts) in &self.receipts {
            if receipt_source != source {
                continue;
            }
            // The signal names one epoch; Events delivered under any other
            // epoch are unaccounted for by it.
            if *receipt_epoch != epoch {
                return true;
            }
            // The source claims fewer emissions than the Core received, which
            // subsumes "reports no emissions at all while Events arrived".
            if receipts.highest > last_emitted {
                return true;
            }
        }
        false
    }
}

/// Evidence Coverage over one Specification's required sources.
///
/// CONTEXT.md defines Evidence Coverage as how much of the evidence a
/// Specification requires Sentinel can establish. This is its source-set half,
/// and it answers as a **conjunction**: a source is covered when the Core both
/// received at least one Event from it *and* holds the [`SourceEvidence`] it
/// declared. Half the evidence is not some of the evidence — a source that
/// delivered Events and declared nothing has said nothing about how much it is
/// still withholding, so the Core cannot state a Watermark, a gap set, a
/// capability set or a tier for it.
///
/// The invariant is **carried, not asserted**. `covered` maps each covered
/// source to its record, so a reader cannot name a covered source and then fail
/// to find its evidence: there is no lookup left to fail.
/// [`EvidenceLedger::coverage`] is the only constructor, and the fields are
/// private, so the two halves cannot be recombined wrongly by a caller either.
#[derive(Clone, Debug, PartialEq)]
pub struct Coverage<'ledger> {
    required: BTreeSet<SourceIdentity>,
    covered: BTreeMap<SourceIdentity, &'ledger SourceEvidence>,
    undeclared: BTreeSet<SourceIdentity>,
}

impl<'ledger> Coverage<'ledger> {
    /// Returns the Specification's required sources.
    #[must_use]
    pub const fn required(&self) -> &BTreeSet<SourceIdentity> {
        &self.required
    }

    /// Returns each covered source together with the evidence it declared.
    #[must_use]
    pub const fn covered(&self) -> &BTreeMap<SourceIdentity, &'ledger SourceEvidence> {
        &self.covered
    }

    /// Returns the required sources that delivered Events but declared nothing.
    ///
    /// Distinct from "uncovered": a required source that delivered no Event and
    /// declared no evidence is uncovered but not *undeclared*. The distinction
    /// is the whole diagnostic value — an undeclared source is one the driver
    /// reached [`EvidenceLedger::record_delivery`] for and never reached
    /// [`EvidenceLedger::record_source`] for.
    #[must_use]
    pub const fn undeclared(&self) -> &BTreeSet<SourceIdentity> {
        &self.undeclared
    }

    /// Returns one source's declared evidence, when that source is covered.
    #[must_use]
    pub fn record(&self, source: &SourceIdentity) -> Option<&'ledger SourceEvidence> {
        self.covered.get(source).copied()
    }

    /// Returns whether one source is covered.
    #[must_use]
    pub fn is_covered(&self, source: &SourceIdentity) -> bool {
        self.covered.contains_key(source)
    }

    /// Returns whether every required source is covered.
    ///
    /// Comparing cardinalities **is** comparing the sets here:
    /// [`EvidenceLedger::coverage`] draws `covered`'s keys from `required` and
    /// from nowhere else, so `covered` can only be a subset and equal sizes
    /// leave no room for a difference.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.covered.len() == self.required.len()
    }

    /// Returns whether no required source is covered at all.
    ///
    /// Read by [`crate::evaluator::Evaluator`] because `sentinel_model` admits
    /// no Verdict without coverage: `AssuranceValidator::check` accepts no
    /// authority whose `hot_path` is not `SETTLED`, and a `SETTLED` tier with
    /// an empty settlement Watermark set is rejected in turn — while the
    /// Watermarks are drawn from [`Coverage::covered`] and nowhere else. The
    /// Core's own `hot_path: SETTLED` literal is downstream of that rule rather
    /// than the reason for it. See [`crate::CoreError::NoEvidenceCoverage`].
    ///
    /// Vacuously true for a Specification that requires no source, which is a
    /// different defect with a different repair:
    /// [`crate::CoreError::SpecificationRequiresNoSource`].
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.covered.is_empty()
    }
}

/// Why required identities, the ledger, or Frontier G4 closed the evidence gate.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum GateReason {
    /// G1: a required source supplied no Event.
    UnobservedRequiredSource,
    /// G1b: a required source delivered Events but declared no evidence.
    UndeclaredRequiredSource,
    /// G2: a required source's delivery sequence is discontinuous.
    RequiredSourceGap,
    /// G3: a required source is unhealthy.
    UnhealthyRequiredSource,
    /// G4: an Event was delivered but never became causally ready.
    HeldEventOrUnresolvedEdge,
    /// G5: the Core has not received every Event up to a declared boundary.
    IncompleteDeliveryToWatermark,
}

impl GateReason {
    /// Returns a stable, human-readable description.
    #[must_use]
    pub const fn detail(self) -> &'static str {
        match self {
            Self::UnobservedRequiredSource => "G1 a required source supplied no Event",
            Self::UndeclaredRequiredSource => {
                "G1b a required source delivered Events but declared no evidence"
            }
            Self::RequiredSourceGap => "G2 a required source's delivery sequence has a gap",
            Self::UnhealthyRequiredSource => "G3 a required source is unhealthy",
            Self::HeldEventOrUnresolvedEdge => {
                "G4 an Event was delivered but never became causally ready"
            }
            Self::IncompleteDeliveryToWatermark => {
                "G5 the Core has not received every Event up to a required source's boundary"
            }
        }
    }
}

/// Whether required identities, the ledger, and Frontier G4 close the evidence gate.
///
/// G1–G5 are the disjuncts collected in [`EvidenceGate::reasons`].
/// [`EvidenceGate::watermarks_cover_epochs`] is Coverage completeness — a second
/// conjunct on this type, not a sixth [`GateReason`]. [`EvidenceGate::is_open`]
/// is empty reasons; it does not incorporate that flag.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct EvidenceGate {
    reasons: BTreeSet<GateReason>,
    unobserved: BTreeSet<SourceIdentity>,
    watermarks_cover_epochs: bool,
}

impl EvidenceGate {
    /// Evaluates the gate over required identities, the ledger, and the Frontier.
    ///
    /// The first argument is the same set [`EvidenceLedger::coverage`] takes.
    /// Reading the ledger rather than an Adapter batch is what makes the gate
    /// evaluable in a resumable Core at all: completeness signals, gaps and
    /// observed-source membership can come from neither a clock nor a store, so
    /// they have to be inputs.
    #[must_use]
    pub fn evaluate(
        required: BTreeSet<SourceIdentity>,
        ledger: &EvidenceLedger,
        frontier: &Frontier,
    ) -> Self {
        let mut reasons = BTreeSet::new();
        let mut unobserved = BTreeSet::new();
        let mut watermarks_cover_epochs = true;
        // The same population `crate::verdict` writes into the envelope and
        // `Evaluator::build_verdicts` reads coverage completeness off. G1 was
        // already the conjunction of deliveries and a record; since #158 it is
        // the conjunction *expressed once*, so the three cannot drift apart.
        let coverage = ledger.coverage(required);

        for source in coverage.required() {
            if let Some(record) = coverage.record(source) {
                if !record.is_contiguous() {
                    reasons.insert(GateReason::RequiredSourceGap);
                }
                if ledger.contradicts_delivery(source) {
                    reasons.insert(GateReason::UnhealthyRequiredSource);
                }
                let completeness = record.completeness();
                if completeness.complete_through().is_none()
                    || completeness.complete_through() != completeness.last_emitted()
                {
                    watermarks_cover_epochs = false;
                }
                // Contract clause (c). Only evaluated when the source claims
                // contiguity: a legitimate source-side hole means `gaps` is
                // non-empty and G2 has already closed the gate, so (c) can
                // never contradict a truthful source.
                if record.is_contiguous() {
                    let boundary = completeness
                        .complete_through()
                        .map_or(0, sentinel_model::SequenceNumber::value);
                    if !ledger.covers_watermark(source, completeness.epoch().value(), boundary) {
                        reasons.insert(GateReason::IncompleteDeliveryToWatermark);
                        watermarks_cover_epochs = false;
                    }
                }
            } else {
                // G1 and G1b are the two ways to fail the conjunction, and they
                // are separate disjuncts because they call for different
                // repairs: G1 is missing instrumentation, G1b is a driver that
                // delivered Events and never sent the source's
                // `SourceEvidence`. Collapsing them would put an untrue
                // sentence -- "supplied no Event" -- into the Assurance
                // Envelope of a Verdict whose source supplied plenty.
                reasons.insert(if coverage.undeclared().contains(source) {
                    GateReason::UndeclaredRequiredSource
                } else {
                    GateReason::UnobservedRequiredSource
                });
                unobserved.insert(source.clone());
                watermarks_cover_epochs = false;
            }
        }

        if !frontier.held().is_empty() || !frontier.unresolved_causal_edges().is_empty() {
            reasons.insert(GateReason::HeldEventOrUnresolvedEdge);
        }

        Self {
            reasons,
            unobserved,
            watermarks_cover_epochs,
        }
    }

    /// Returns whether the gate is open: no [`GateReason`] fired.
    ///
    /// This is empty [`EvidenceGate::reasons`], not
    /// [`EvidenceGate::watermarks_cover_epochs`].
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.reasons.is_empty()
    }

    /// Returns the disjuncts that fired, in a stable order.
    #[must_use]
    pub const fn reasons(&self) -> &BTreeSet<GateReason> {
        &self.reasons
    }

    /// Returns the required sources Evidence Coverage does not cover.
    ///
    /// Named for the common case rather than the whole of it: a source that
    /// delivered Events but declared no evidence is uncovered too, and appears
    /// here. That is **not** a change of population — the pre-#158 catch-all
    /// arm already collected such a source — only of the [`GateReason`] label
    /// recorded beside it, so there is no migration to go looking for.
    #[must_use]
    pub const fn unobserved_sources(&self) -> &BTreeSet<SourceIdentity> {
        &self.unobserved
    }

    /// Returns whether every required source's Watermark covers its full epoch.
    ///
    /// Coverage completeness, a second conjunct on this type — not a sixth
    /// [`GateReason`].
    #[must_use]
    pub const fn watermarks_cover_epochs(&self) -> bool {
        self.watermarks_cover_epochs
    }

    /// Returns a deterministic description of every disjunct that fired.
    #[must_use]
    pub fn detail(&self) -> String {
        self.reasons
            .iter()
            .map(|reason| reason.detail())
            .collect::<Vec<_>>()
            .join("; ")
    }
}
