//! Per-source evidence: tier, capabilities, completeness signal, and gaps.
//!
//! These types were built by the Adapter in `sentinel_ingest` and consumed by
//! the Verification Core. Issue #12 moved them here because they are pure
//! model-built validated types — everything in them is assembled out of
//! [`SourceTier`], [`SourceCapabilities`] and [`SourceCompletenessSignal`],
//! all of which already live in this crate — and because the Core's resumable
//! evaluator takes [`SourceEvidence`] as an *input* rather than reading it out
//! of an Adapter batch.
//!
//! **`ObservedEvidence` deliberately did not move.** It is an Adapter *batch
//! handoff* shape whose `with_events` exists so a test can permute arrival
//! order; a batch is not a domain type and it belongs at the Adapter boundary.
//! Only the per-source half is canonical.

#![allow(clippy::module_name_repetitions)]

use crate::{SourceCapabilities, SourceCompletenessSignal, SourceIdentity, SourceTier};

/// A detected discontinuity in one source's delivery sequence.
///
/// The gap is computed by the **Adapter**, over the sequences the Adapter
/// itself emitted. It is a claim about what the source *produced*, never about
/// what a receiver *received*. Anything that needs the latter has to derive it
/// from its own receipts (see `sentinel_core`'s evidence ledger).
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct SequenceGap {
    /// Sequence epoch the gap falls in.
    pub sequence_epoch: u64,
    /// First missing sequence number, inclusive.
    pub first_missing: u64,
    /// Last missing sequence number, inclusive.
    pub last_missing: u64,
}

impl SequenceGap {
    /// Records one discontinuity in a source's emitted sequence.
    #[must_use]
    pub const fn new(sequence_epoch: u64, first_missing: u64, last_missing: u64) -> Self {
        Self {
            sequence_epoch,
            first_missing,
            last_missing,
        }
    }
}

/// Everything known about one Event source in this evidence set.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SourceEvidence {
    source: SourceIdentity,
    tier: SourceTier,
    capabilities: SourceCapabilities,
    completeness: SourceCompletenessSignal,
    gaps: Vec<SequenceGap>,
}

impl SourceEvidence {
    /// Records the evidence one source supplied.
    #[must_use]
    pub fn new(
        source: SourceIdentity,
        tier: SourceTier,
        capabilities: SourceCapabilities,
        completeness: SourceCompletenessSignal,
        gaps: Vec<SequenceGap>,
    ) -> Self {
        let mut gaps = gaps;
        gaps.sort_unstable();
        gaps.dedup();
        Self {
            source,
            tier,
            capabilities,
            completeness,
            gaps,
        }
    }

    /// Returns the source identity.
    #[must_use]
    pub const fn source(&self) -> &SourceIdentity {
        &self.source
    }

    /// Returns the deployment-configured assurance tier (ADR-0022).
    ///
    /// This is the tier the source is *placed* at, which ADR-0022 makes a
    /// property of the deployed source rather than of the Specification. A
    /// Specification declares the *minimum* tier it needs; the evidence
    /// declares what actually arrived.
    #[must_use]
    pub const fn tier(&self) -> SourceTier {
        self.tier
    }

    /// Returns the effective verified capabilities.
    #[must_use]
    pub const fn capabilities(&self) -> &SourceCapabilities {
        &self.capabilities
    }

    /// Returns the synthesized source completeness signal.
    #[must_use]
    pub const fn completeness(&self) -> &SourceCompletenessSignal {
        &self.completeness
    }

    /// Returns detected sequence gaps, in ascending order.
    #[must_use]
    pub fn gaps(&self) -> &[SequenceGap] {
        &self.gaps
    }

    /// Returns whether the source claims it emitted a contiguous sequence.
    ///
    /// Contiguity is the **source's** claim about its own emissions. It is not
    /// evidence that a receiver holds every Event below the boundary.
    #[must_use]
    pub fn is_contiguous(&self) -> bool {
        self.gaps.is_empty()
    }
}
