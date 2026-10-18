//! The one bounded, data-parametric Specification shape this slice evaluates.
//!
//! These are plain data types, not a Pattern catalogue and not a compiler. Issue
//! #14/#15 own the catalogue and the Specification DSL; ADR-0019 wants a few
//! high-value Patterns derived from partner evidence, not a catalogue derived
//! from a testbed. Nothing here is a Catalogue entry.
//!
//! [`PrecedenceSpecification`] lives in `core/` because it is what the Evaluator
//! steps, and evaluator state cannot move into `compile`. The crate graph
//! forces the same direction for every family: `core` already depends on
//! `sentinel-compile`, so the reverse edge is a cycle. A Pattern lowers to
//! plain data in `compile`. The Core reads the precedence family here via
//! `TryFrom<&PrecedenceIr>` (and the taint family via `TryFrom<&TaintIr>` in
//! `taint.rs`). `compile` is the producer of `MonitorIr`, not a second
//! consumer of these types.
//!
//! Do not read [`crate::taint::TaintSpecification`] as a polarity flag on
//! [`PrecedenceSpecification`]. It is a sibling Core type with its own table;
//! [`crate::evaluator::Evaluator`] hides both families behind a private
//! `Monitored` enum. Dispatch is not a public Core `Specification` dual.
//!
//! The rejected move — Specification types into `compile`, re-exported from
//! `core` — is recorded in `compile/src/ir.rs`. Issue #142 closed it.
//!
//! **No SagaShop-shaped constructor lives in this module.** The testbed's
//! `ship_requires_prior_charge()` is a test fixture and lives in
//! `core/tests/support/mod.rs`; putting it in a production crate would make
//! #14's first task deleting it.
//!
//! # What "bounded" and "data-parametric" mean concretely
//!
//! **Bounded:** past-time only, with no future obligation. Per-Slice state is
//! bounded by the Slice's own Events and does not grow with the global Trace.
//! The verdict is read off the frontier valuation with no linearization ever
//! materialised, at a per-Binding cost of `O(|Slice| * |triggers|)` reachability
//! queries. This sits inside ADR-0020's PaBTL-class fragment; nothing here needs
//! the reachability class ADR-0020 assigns to the cold path.
//!
//! **Data-parametric:** the Specification has one free variable, bound to an
//! Event attribute. Each Binding gets its own Slice and its own Verdict.

#![allow(clippy::doc_markdown)] // Product names (SagaShop, OpenTelemetry) read as prose here.
#![allow(clippy::module_name_repetitions)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use sentinel_compile::ir::{PrecedenceIr, RetirementIr, SelectorIr};
use sentinel_model::{
    ActivationBarrier, AttributeValue, Event, EventKind, IdentifierError, ProducerEvent,
    SchemaVersion, SourceIdentity, SourceTier,
};

use crate::facts::ColdGuardId;
use crate::slice::RetirementCause;

pub use sentinel_model::AttributeKey;

// Sibling of `sentinel_model::identifiers::string_identifier!`: the same
// emptiness rule (`trim().is_empty()` → `IdentifierError`) and the same
// untrimmed storage (`"  foo"` succeeds). Kept local so Specification
// identity does not live in the Event / source / clock identifier module.
macro_rules! core_string_identifier {
    ($(#[$metadata:meta])* $name:ident) => {
        $(#[$metadata])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Parses and validates an identifier.
            ///
            /// # Errors
            ///
            /// Returns [`IdentifierError`] when `value` is empty or whitespace.
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(IdentifierError);
                }
                Ok(Self(value))
            }

            /// Returns the identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdentifierError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = IdentifierError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    };
}

core_string_identifier!(
    /// Stable identity of one Specification's text.
    SpecificationId
);
core_string_identifier!(
    /// Version of one Specification's text.
    SpecificationVersion
);

/// Why converting public Pattern IR into a Core Specification failed.
///
/// IR identity and Binding-attribute fields are still public `String`s.
/// [`TryFrom`] names which field was blank rather than folding every refusal
/// into the unit [`IdentifierError`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SpecificationConversionError {
    /// `specification_id` was empty or whitespace.
    BlankSpecificationId,
    /// `specification_version` was empty or whitespace.
    BlankSpecificationVersion,
    /// `binding_attribute` was empty or whitespace.
    BlankBindingAttribute,
}

impl Display for SpecificationConversionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        let field = match self {
            Self::BlankSpecificationId => "specification_id",
            Self::BlankSpecificationVersion => "specification_version",
            Self::BlankBindingAttribute => "binding_attribute",
        };
        write!(
            formatter,
            "{field}: identifier must not be empty or whitespace"
        )
    }
}

impl Error for SpecificationConversionError {}

/// Stable identity of one Specification.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub struct SpecificationIdentity {
    id: SpecificationId,
    version: SpecificationVersion,
}

impl SpecificationIdentity {
    /// Names a Specification and the version of its text.
    ///
    /// Infallible: each half has already refused blank. A `Result` on top of
    /// validated newtypes would be a second check of a type that already
    /// closed the hole.
    #[must_use]
    pub const fn new(id: SpecificationId, version: SpecificationVersion) -> Self {
        Self { id, version }
    }

    /// Returns the Specification identity.
    #[must_use]
    pub fn id(&self) -> &str {
        self.id.as_str()
    }

    /// Returns the Specification version.
    #[must_use]
    pub fn version(&self) -> &str {
        self.version.as_str()
    }
}

/// Selects Events by kind and by required attribute valuations.
///
/// An Event matches when its kind is equal and every required attribute is
/// present with exactly the required value. Absent attributes never match: this
/// slice does not infer a value it did not observe.
///
/// **The matching itself is `sentinel_compile::ir::SelectorIr`'s, not this
/// type's.** This is a thin wrapper that keeps the Core's own vocabulary — a
/// `PrecedenceSpecification` names an `EventSelector`, not an IR node — while
/// leaving exactly one implementation of "does this Event match?" in the
/// workspace. There used to be two, and `compile/src/ir.rs` conceded as much
/// (`SelectorIr` "mirrors `core`'s `EventSelector`"). Issue #144 needs the same
/// question answered inside `compile`, and a second copy that could drift from
/// this one would be a census disagreeing with the evaluator it censuses —
/// exactly the duplication ADR-0027 and issue #112 exist to remove.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct EventSelector {
    selector: SelectorIr,
}

impl EventSelector {
    /// Selects Events of one kind carrying the given attribute valuations.
    #[must_use]
    pub const fn new(
        kind: EventKind,
        required_attributes: BTreeMap<AttributeKey, AttributeValue>,
    ) -> Self {
        Self {
            selector: SelectorIr::new(kind, required_attributes),
        }
    }

    /// Returns the selected Event kind.
    #[must_use]
    pub const fn kind(&self) -> &EventKind {
        &self.selector.kind
    }

    /// Returns the attribute valuations an Event must carry to be selected.
    #[must_use]
    pub const fn required_attributes(&self) -> &BTreeMap<AttributeKey, AttributeValue> {
        &self.selector.required_attributes
    }

    /// Returns whether the producer's kind matches, ignoring attributes.
    #[must_use]
    pub fn matches_kind_producer(&self, producer: &ProducerEvent) -> bool {
        self.selector.matches_kind_producer(producer)
    }

    /// Returns whether the Event's kind matches, ignoring attributes.
    #[must_use]
    pub fn matches_kind(&self, event: &Event) -> bool {
        self.selector.matches_kind(event)
    }

    /// Returns whether the producer matches kind and every required attribute.
    #[must_use]
    pub fn matches_producer(&self, producer: &ProducerEvent) -> bool {
        self.selector.matches_producer(producer)
    }

    /// Returns whether the Event matches kind and every required attribute.
    #[must_use]
    pub fn matches(&self, event: &Event) -> bool {
        self.selector.matches(event)
    }
}

/// A source the Specification's conclusion is not trustworthy without.
///
/// Ordering is by source identity alone, so a required-source set holds one
/// entry per source and a tier can never be silently declared twice.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct RequiredSource {
    source: SourceIdentity,
    tier: SourceTier,
}

impl Ord for RequiredSource {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.source.cmp(&other.source)
    }
}

impl PartialOrd for RequiredSource {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl RequiredSource {
    /// Declares a required source and the assurance tier it must hold.
    #[must_use]
    pub const fn new(source: SourceIdentity, tier: SourceTier) -> Self {
        Self { source, tier }
    }

    /// Returns the required source identity.
    #[must_use]
    pub const fn source(&self) -> &SourceIdentity {
        &self.source
    }

    /// Returns the **minimum** assurance tier this conclusion needs.
    ///
    /// ADR-0022 keeps the *required* tier a Specification property the compiler
    /// enforces at promotion, while tier *placement* is a deployment and
    /// assurance fact the evidence asserts. So this is what the Specification
    /// demands ("this conclusion is not trustworthy without an anchor-tier
    /// source for X"), and `SourceEvidence::tier()` is what actually arrived.
    /// A source arriving below its declared minimum needs no new gate reason:
    /// it simply supplies no anchor candidates, and the existing anchor
    /// downgrade demotes the Violate on its own.
    #[must_use]
    pub const fn minimum_tier(&self) -> SourceTier {
        self.tier
    }
}

/// A global-scope precedence obligation over one Binding attribute.
///
/// The obligation is: for every trigger Event `t` in a Binding's Slice, there
/// exists a requirement Event `w` in the same Slice with `w ->* t` in the
/// happens-before relation. This is the Dwyer precedence Pattern at global
/// scope, and exactly the causal-past check ADR-0020 names from CPL.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct PrecedenceSpecification {
    identity: SpecificationIdentity,
    binding_attribute: AttributeKey,
    trigger: EventSelector,
    requirement: EventSelector,
    required_sources: BTreeSet<RequiredSource>,
    schema_version: SchemaVersion,
    cold_guards: BTreeSet<ColdGuardId>,
    activation_barrier: Option<ActivationBarrier>,
    retirement_policy: RetirementPolicy,
}

/// What a Specification declares about a Scope that may never close.
///
/// The Core's own dual of `sentinel_compile::ir::RetirementIr`, which is in turn
/// the IR dual of the catalogue's `RetirementPolicy`. Three types rather than
/// one re-exported enum, for the reason `compile/src/ir.rs` gives for every
/// other field: the Core reads IR, not catalogue vocabulary, and a Core type is
/// what its own evaluator matches on.
///
/// It is read by exactly one thing — clause (d0) of the retirement
/// admissibility contract in [`crate::evaluator`] — and it is what makes "a
/// CAT-4 Binding cannot be retired" a consequence of the Pattern's own
/// declaration rather than a family check written into the evaluator. A
/// hand-built Specification defaults to [`RetirementPolicy::NotRequired`], so
/// every existing fixture keeps refusing retirement without moving.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum RetirementPolicy {
    /// The Scope cannot fail to close, so no retirement signal is admissible.
    #[default]
    NotRequired,
    /// The Scope retires when the periphery's declared deadline passes.
    Deadline,
    /// The Scope retires after the periphery's declared quiet window.
    Inactivity,
}

impl RetirementPolicy {
    /// Returns the retirement cause this policy admits, if it admits one.
    ///
    /// `None` is clause (d0)'s first refusal. When it is `Some`, clause (d0)'s
    /// second refusal is that the signal's cause is a different one — the
    /// Specification declares exactly one policy, so exactly one cause is
    /// admissible on any given evaluator.
    #[must_use]
    pub const fn declared_cause(self) -> Option<RetirementCause> {
        match self {
            Self::NotRequired => None,
            Self::Deadline => Some(RetirementCause::Deadline),
            Self::Inactivity => Some(RetirementCause::Inactivity),
        }
    }
}

/// Reads a lowered Pattern's retirement declaration into the Core's own enum.
///
/// An IR variant this Core does not know maps to
/// [`RetirementPolicy::NotRequired`]. `RetirementIr` is `#[non_exhaustive]`, so
/// the wildcard is unavoidable here, and refusing retirement is the
/// conservative direction: the failure mode is a Scope that stays open and
/// reports prefix uncertainty, never one that retires on a policy this Core
/// cannot read.
impl From<RetirementIr> for RetirementPolicy {
    fn from(lowered: RetirementIr) -> Self {
        match lowered {
            RetirementIr::Deadline => Self::Deadline,
            RetirementIr::Inactivity => Self::Inactivity,
            // `RetirementIr::NotRequired`, and every variant a later `compile`
            // adds. Folded together deliberately: refusing retirement is the
            // conservative answer for a policy this Core cannot read.
            _ => Self::NotRequired,
        }
    }
}

impl PrecedenceSpecification {
    /// Declares a precedence obligation parameterised by one Event attribute.
    #[must_use]
    pub fn new(
        identity: SpecificationIdentity,
        binding_attribute: AttributeKey,
        trigger: EventSelector,
        requirement: EventSelector,
        required_sources: BTreeSet<RequiredSource>,
        schema_version: SchemaVersion,
    ) -> Self {
        Self {
            identity,
            binding_attribute,
            trigger,
            requirement,
            required_sources,
            schema_version,
            cold_guards: BTreeSet::new(),
            activation_barrier: None,
            retirement_policy: RetirementPolicy::NotRequired,
        }
    }

    /// Declares the cold-path guards this Specification's conclusion needs.
    ///
    /// Empty for a purely hot-path Specification, which is what makes the
    /// guard-free settlement row identical to #10's.
    #[must_use]
    pub fn with_cold_guards(mut self, cold_guards: BTreeSet<ColdGuardId>) -> Self {
        self.cold_guards = cold_guards;
        self
    }

    /// Returns the declared cold-path guards.
    #[must_use]
    pub const fn cold_guards(&self) -> &BTreeSet<ColdGuardId> {
        &self.cold_guards
    }

    /// Declares the activation barrier this Specification is warming behind.
    ///
    /// `None` -- the default -- is a Specification that declares no barrier, which is
    /// every Specification `sentinel_compile::filter::compile_filter` emits today. The
    /// barrier is read out of a published Event Filter with
    /// [`sentinel_model::ActivationBarrier::try_from_event_filter`], **not** threaded
    /// through `sentinel_compile::ir::PrecedenceIr`: `compile/src/filter.rs`'s own module
    /// doc says the Pattern IR "is NOT the proto `SpecificationFilter` -- that mixes
    /// relevance with enforcement state (#19)", so rollout state does not belong in the
    /// IR.
    ///
    /// Additive beside [`PrecedenceSpecification::with_cold_guards`], and
    /// [`PrecedenceSpecification::new`] is unchanged, so no existing call site or fixture
    /// moves. The `Eq` derive is retained: [`sentinel_model::ObservationTime`] carries a
    /// manual `impl Eq`, so `Option<ActivationBarrier>` is `Eq` and the derive still
    /// resolves.
    #[must_use]
    pub fn with_activation_barrier(mut self, activation_barrier: ActivationBarrier) -> Self {
        self.activation_barrier = Some(activation_barrier);
        self
    }

    /// Declares what this Specification says about a Scope that may never close.
    ///
    /// Additive beside [`PrecedenceSpecification::with_activation_barrier`], in
    /// the same shape and for the same reason: [`PrecedenceSpecification::new`]
    /// is unchanged, so no existing call site or fixture moves and every
    /// hand-built Specification keeps the default
    /// [`RetirementPolicy::NotRequired`].
    ///
    /// Unlike the activation barrier, this **is** threaded through the IR: a
    /// retirement policy is a property of the Pattern, declared in the
    /// catalogue, whereas a barrier is rollout state that `compile/src/filter.rs`
    /// deliberately keeps out of the IR. `TryFrom<&PrecedenceIr>` therefore sets
    /// it, and this builder exists for the hand-built Specifications tests use.
    #[must_use]
    pub const fn with_retirement_policy(mut self, retirement_policy: RetirementPolicy) -> Self {
        self.retirement_policy = retirement_policy;
        self
    }

    /// Returns what this Specification declares about a never-closing Scope.
    ///
    /// Read by clause (d0) of [`crate::evaluator`]'s retirement admissibility
    /// contract, and by nothing else.
    #[must_use]
    pub const fn retirement_policy(&self) -> RetirementPolicy {
        self.retirement_policy
    }

    /// Returns the activation barrier, when this Specification declares one.
    ///
    /// Read by [`crate::settlement::BarrierState::of`], which is the only thing that turns
    /// it into a suppression decision. Nothing else in the Core reads it: the evidence
    /// gate is deliberately untouched, so a warming Specification emits from
    /// [`crate::evaluator::Evaluator::step`] exactly as it does today rather than
    /// returning `Pending` for its entire warming period.
    #[must_use]
    pub const fn activation_barrier(&self) -> Option<&ActivationBarrier> {
        self.activation_barrier.as_ref()
    }

    /// Returns the Specification identity.
    #[must_use]
    pub const fn identity(&self) -> &SpecificationIdentity {
        &self.identity
    }

    /// Returns the attribute whose value names a Binding.
    #[must_use]
    pub const fn binding_attribute(&self) -> &AttributeKey {
        &self.binding_attribute
    }

    /// Returns the selector for Events that create an obligation.
    #[must_use]
    pub const fn trigger(&self) -> &EventSelector {
        &self.trigger
    }

    /// Returns the selector for Events that discharge an obligation.
    #[must_use]
    pub const fn requirement(&self) -> &EventSelector {
        &self.requirement
    }

    /// Returns the sources the conclusion is not trustworthy without.
    #[must_use]
    pub const fn required_sources(&self) -> &BTreeSet<RequiredSource> {
        &self.required_sources
    }

    /// Returns the required source identities.
    #[must_use]
    pub fn required_source_identities(&self) -> BTreeSet<SourceIdentity> {
        self.required_sources
            .iter()
            .map(|required| required.source().clone())
            .collect()
    }

    /// Returns the minimum assurance tier declared for one required source.
    ///
    /// This is **not** the tier a Verdict records. The recorded tier is read
    /// from `SourceEvidence::tier()` through the evidence ledger, per ADR-0022.
    #[must_use]
    pub fn minimum_tier_of(&self, source: &SourceIdentity) -> Option<SourceTier> {
        self.required_sources
            .iter()
            .find(|required| required.source() == source)
            .map(RequiredSource::minimum_tier)
    }

    /// Returns the producer schema version this Specification is written against.
    #[must_use]
    pub const fn schema_version(&self) -> &SchemaVersion {
        &self.schema_version
    }

    /// Returns the Binding producer evidence belongs to, when it carries the
    /// free variable.
    ///
    /// Delegates to [`sentinel_compile::ir::binding_of_producer`], the one
    /// implementation of the Binding-must-be-`String` rule.
    #[must_use]
    pub fn binding_of_producer(&self, producer: &ProducerEvent) -> Option<String> {
        sentinel_compile::ir::binding_of_producer(&self.binding_attribute, producer)
    }

    /// Returns the Binding an Event belongs to, when it carries the free variable.
    ///
    /// **The rule itself is `sentinel_compile::ir::binding_of`'s, not this
    /// type's**, for the same reason [`EventSelector`] delegates its matching:
    /// issue #144 asks `compile` whether a Pattern instance raised any
    /// obligation, and that question is unanswerable without this one. A second
    /// copy over there would be a census that can disagree with the evaluator it
    /// censuses — and it would disagree on a case easy to get wrong, since a
    /// Binding attribute present but holding a non-`String` value names no
    /// Binding at all.
    #[must_use]
    pub fn binding_of(&self, event: &Event) -> Option<String> {
        sentinel_compile::ir::binding_of(&self.binding_attribute, event)
    }
}

/// Lowers a catalogued Pattern's precedence IR into the Core's own Specification.
///
/// This is the one direction the crate graph permits. `compile` cannot depend on
/// `core` — `core` already depends on `sentinel-compile`, so the reverse edge
/// would be a cycle — which is why a Pattern lowers to plain data in `compile`
/// and the Core reads it here.
///
/// **Fallible, because IR fields are still public `String`s.**
/// `sentinel_compile::catalogue::instantiate` refuses a blank parameter before
/// an IR exists, so a catalogued instance cannot take this error path. Public
/// IR fields can still be cloned and assigned `"   "` after instantiate; that
/// used to `From` into a Specification whose `evaluate` returned `Ok(vec![])`
/// — a silent pass. Conversion now returns [`SpecificationConversionError`]
/// instead of constructing, so nothing calls `evaluate` on the poisoned IR.
///
/// A well-formed but simply wrong attribute key — `sagashop.order.i` for
/// `sagashop.order.id` — still converts: whether a key names anything is a
/// fact about the evidence, not about the Specification. That residue is
/// caught where the evidence is: `core/tests/catalogue_grounding.rs`
/// classifies a Binding-free evaluation as `NoBindings` rather than folding
/// it into a pass.
///
/// Cold guards are deliberately not carried. The IR has none to carry: CAT-1 is
/// declared hot with zero expected fact requests per Verdict, so a lowering that
/// produced a guard would contradict the Pattern's own Cost Class. The Pattern
/// that needs guards will extend the IR, not this lowering.
///
/// The **retirement policy is** carried, and that is the difference between a
/// Pattern property and rollout state. What a Pattern declares about a Scope
/// that may never close is catalogue data, so it crosses; whether a deployment
/// is warming behind a Filter epoch is not, so the activation barrier does not.
///
/// Note what does **not** cross: the Core sees no `PatternId`, no display name,
/// no width budget, no grounding reference and no evidence grade. It evaluates
/// one Specification and has no business knowing which template produced it.
impl TryFrom<&PrecedenceIr> for PrecedenceSpecification {
    type Error = SpecificationConversionError;

    fn try_from(lowered: &PrecedenceIr) -> Result<Self, Self::Error> {
        let id = SpecificationId::new(lowered.specification_id.clone())
            .map_err(|_| SpecificationConversionError::BlankSpecificationId)?;
        let version = SpecificationVersion::new(lowered.specification_version.clone())
            .map_err(|_| SpecificationConversionError::BlankSpecificationVersion)?;
        let binding_attribute = AttributeKey::new(lowered.binding_attribute.clone())
            .map_err(|_| SpecificationConversionError::BlankBindingAttribute)?;
        Ok(Self::new(
            SpecificationIdentity::new(id, version),
            binding_attribute,
            EventSelector::new(
                lowered.trigger.kind.clone(),
                lowered.trigger.required_attributes.clone(),
            ),
            EventSelector::new(
                lowered.requirement.kind.clone(),
                lowered.requirement.required_attributes.clone(),
            ),
            lowered
                .required_sources
                .iter()
                .map(|(source, tier)| RequiredSource::new(source.clone(), *tier))
                .collect(),
            lowered.schema_version.clone(),
        )
        .with_retirement_policy(RetirementPolicy::from(lowered.retirement)))
    }
}

#[cfg(test)]
mod tests {
    use super::{SpecificationId, SpecificationVersion};
    use sentinel_model::IdentifierError;

    #[test]
    fn specification_identity_halves_reject_empty_and_whitespace() {
        assert_eq!(SpecificationId::new(""), Err(IdentifierError));
        assert_eq!(SpecificationId::new("   "), Err(IdentifierError));
        assert_eq!(SpecificationVersion::new(""), Err(IdentifierError));
        assert_eq!(SpecificationVersion::new("   "), Err(IdentifierError));
    }

    #[test]
    fn specification_identity_halves_store_untrimmed_non_blank_text() {
        assert_eq!(
            SpecificationId::new(" foo").expect("non-blank").as_str(),
            " foo"
        );
        assert_eq!(
            SpecificationId::new("  foo").expect("non-blank").as_str(),
            "  foo"
        );
        assert_eq!(
            SpecificationVersion::new(" foo")
                .expect("non-blank")
                .as_str(),
            " foo"
        );
        assert_eq!(
            SpecificationVersion::new("  foo")
                .expect("non-blank")
                .as_str(),
            "  foo"
        );
    }

    #[test]
    fn specification_conversion_error_names_the_field_and_the_identifier_rule() {
        assert_eq!(
            super::SpecificationConversionError::BlankSpecificationId.to_string(),
            "specification_id: identifier must not be empty or whitespace"
        );
        assert_eq!(
            super::SpecificationConversionError::BlankSpecificationVersion.to_string(),
            "specification_version: identifier must not be empty or whitespace"
        );
        assert_eq!(
            super::SpecificationConversionError::BlankBindingAttribute.to_string(),
            "binding_attribute: identifier must not be empty or whitespace"
        );
    }
}
