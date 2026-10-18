//! Fact-request counting: the measured actual, not the declared Cost Class.
//!
//! ADR-0012's consequence "fact requests per Verdict are directly countable" is
//! what this module implements. It is **not** the Cost Class: CONTEXT.md
//! defines a Cost Class as the tier and expected per-Event cost "known from its
//! Pattern *before* deployment", which is a compiler declaration and #14/#15's
//! job. What #12 supplies is the *measured actual* that such a declaration will
//! be scored against, exactly mirroring ADR-0008's "actual width versus
//! declared budget" framing for Filter width.
//!
//! **Explicitly not on the wire and not in the `AssuranceEnvelope`.** The
//! envelope carries the *evidence conditions* a Verdict rests on; how many
//! round trips it took is operational cost, not an evidence condition. It sits
//! beside `Explanation` as a read-side structure, which is the pattern
//! `crate::verdict` already establishes.
//!
//! `round_trips` is **evaluator-level only**. One round trip serves many
//! Bindings, so a per-Binding round-trip count would not sum to the evaluator's
//! total; per-Binding attribution is by `round_trip_indices` instead.
//!
//! `queries_issued` counts **re-issues**, and `responses_applied` counts every
//! accepted response including the ones that did not settle. That is the honest
//! cost signal: a guard that has to be asked three times costs three round
//! trips, and ADR-0012 is about requests, not about successful ones.

#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};

use crate::slice::Binding;

/// What one Binding's Verdict cost in cold-path round trips.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct BindingFactCost {
    queries_issued: u32,
    responses_applied: u32,
    round_trip_indices: BTreeSet<u32>,
}

impl BindingFactCost {
    /// Returns how many queries were issued for this Binding, re-issues included.
    #[must_use]
    pub const fn queries_issued(&self) -> u32 {
        self.queries_issued
    }

    /// Returns how many responses were accepted for this Binding.
    #[must_use]
    pub const fn responses_applied(&self) -> u32 {
        self.responses_applied
    }

    /// Returns the round trips this Binding's queries took part in.
    #[must_use]
    pub const fn round_trip_indices(&self) -> &BTreeSet<u32> {
        &self.round_trip_indices
    }
}

/// Cold-path cost for one evaluation, in total and per Binding.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct EvaluationCost {
    round_trips: u32,
    queries_issued: u32,
    responses_applied: u32,
    per_binding: BTreeMap<Binding, BindingFactCost>,
}

impl EvaluationCost {
    /// Returns how many batched round trips were issued.
    #[must_use]
    pub const fn round_trips(&self) -> u32 {
        self.round_trips
    }

    /// Returns how many queries were issued in total, re-issues included.
    #[must_use]
    pub const fn queries_issued(&self) -> u32 {
        self.queries_issued
    }

    /// Returns how many responses were accepted in total.
    #[must_use]
    pub const fn responses_applied(&self) -> u32 {
        self.responses_applied
    }

    /// Returns the cost recorded for one Binding.
    #[must_use]
    pub fn binding(&self, binding: &Binding) -> Option<&BindingFactCost> {
        self.per_binding.get(binding)
    }

    /// Returns the per-Binding cost table.
    #[must_use]
    pub const fn per_binding(&self) -> &BTreeMap<Binding, BindingFactCost> {
        &self.per_binding
    }

    /// Records one issued query for one Binding, in one round trip.
    pub fn record_issue(&mut self, binding: &Binding, round_trip: u32) {
        self.queries_issued += 1;
        let entry = self.per_binding.entry(binding.clone()).or_default();
        entry.queries_issued += 1;
        entry.round_trip_indices.insert(round_trip);
    }

    /// Records one accepted response for one Binding.
    pub fn record_response(&mut self, binding: &Binding) {
        self.responses_applied += 1;
        self.per_binding
            .entry(binding.clone())
            .or_default()
            .responses_applied += 1;
    }

    /// Records that one batched round trip was issued.
    pub fn record_round_trip(&mut self) {
        self.round_trips += 1;
    }
}
