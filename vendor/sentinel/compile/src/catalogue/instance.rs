//! Binding a Pattern's parameters to concrete values, and what that produces.
//!
//! A [`PatternInstance`] is what CONTEXT.md calls a Specification before it is
//! promoted: one Pattern, its parameters bound, its evidence roles bound, and
//! the monitor IR it lowers to. It implements
//! [`FilterSource`](crate::FilterSource), which is what
//! `compile/src/filter.rs` calls "the seam #14 Patterns plug into" and what
//! `core/tests/filter_compile.rs` has been standing in for with a test-only
//! newtype "until #14 supplies real Pattern impls".
//!
//! # Where `SchemaVersion` comes from
//!
//! `PrecedenceSpecification::new` requires a `SchemaVersion`, so an
//! instantiation has to get one from somewhere, and the plan for this issue
//! never named where. It is **not** a Pattern parameter: it does not vary with
//! the property being expressed, it is identical for every Pattern written
//! against the same producers, and putting it in the parameter list would ask
//! #21 to generate it and #35 to render a form field for it. It is a
//! producer-contract fact, so it travels in [`ProducerContract`], alongside the
//! Specification identity rather than inside the argument map.
//! `schema_version_is_not_a_pattern_parameter` guards the decision.
//!
//! # What a parameter *value* is checked for, and what it is not
//!
//! [`instantiate`] checks that every declared parameter is bound, that the bound
//! value has the declared kind, and that Specification identity strings are
//! neither empty nor whitespace-only. Attribute-key parameters and selector
//! required-attribute maps are already sealed by [`AttributeKey`]: a blank name
//! is unrepresentable there (issue #203). It does **not** check that an
//! attribute key names an attribute some producer actually emits, and it cannot.
//!
//! The blank check on identity is this repository's existing rule for a
//! producer-supplied string rather than a new one invented here.
//! `sentinel_model::identifiers` rejects exactly `value.trim().is_empty()` for
//! every identifier it defines. Applying it to Specification identity here is
//! not redundancy: the far-end Filter path refuses blank identity with a
//! different error shape that names neither the Pattern nor the parameter, and
//! `core/tests/catalogue_grounding.rs` lowers straight from IR to a
//! Specification and never takes that path at all.
//!
//! A stronger rule was considered and rejected: a well-formedness grammar for
//! attribute keys — dotted lowercase segments, say. Attribute keys are chosen by
//! **producers**, not by this crate. An Event's producer attributes are an
//! arbitrary `BTreeMap<AttributeKey, AttributeValue>`, the wire model admits any
//! non-blank key, and six Probe SDKs emit whatever their host application names
//! its fields. A grammar here would reject keys the wire accepts and the Probes
//! already emit, and a Pattern that cannot express a real deployment's attribute
//! names is a worse failure than the one the grammar would prevent. This crate
//! does not own that namespace and should not legislate it.
//!
//! That leaves a residue no *value* check can close. `sagashop.order.i` is
//! well-formed, non-blank, and wrong: it is `sagashop.order.id` with one
//! character missing. `sagashop.order.ship.requeste` is the same mistake in a
//! trigger selector. Either produces zero Verdicts, which any aggregation
//! written as `Iterator::all` — `true` over an empty slice — reports as a clean
//! pass, and neither is distinguishable from a real key or kind whose producer
//! has not shipped yet.
//!
//! Only evidence separates them, so the catch is
//! [`PatternInstance::validate_against`], which takes evidence a caller asserts
//! is settled and reports what the selectors actually matched. That is a change
//! from what this paragraph said before issue #144: the catch was described as
//! belonging "where the Verdicts are", and half of it does not — the trigger
//! half produces no Verdict to catch anything with. Read `exercise.rs` for the
//! two-level design and for why one Trace's worth of "matched nothing" is a
//! fact rather than a defect.
//!
//! Three halves, then, not two: [`instantiate`] makes the unmeaning key
//! unrepresentable, [`PatternInstance::validate_against`] makes the merely wrong
//! key or kind observable before Promotion, and
//! `core/tests/catalogue_grounding.rs`'s `PropertyOutcome` refuses to fold an
//! empty Verdict vector into a pass after evaluation.
//!
//! # Errors versus warnings
//!
//! A missing, mistyped or unusable parameter is an [`InstanceError`]: the
//! Pattern's meaning is undefined without it. A source that is placed below the tier its
//! role demands is an [`InstanceWarning`], because ADR-0022 makes the required
//! tier a Specification property and the placement a **deployment fact** — the
//! same split `core/src/specification.rs` states on `RequiredSource`. Refusing
//! to instantiate would be this crate overruling a deployment it cannot see; a
//! warning says the conclusion will not be authoritative and lets the operator
//! decide. SagaShop P7 is exactly this case, and the caveat currently lives in
//! English in a test comment.
//!
//! The census warnings issue #144 adds are on the same side of that split and
//! for the same reason: whether a deployment ever emits an Event kind is a fact
//! about the deployment. A Specification naming a kind nobody emits today is
//! well-formed, and the Pattern that will be right the moment the producer ships
//! must not be refused now.

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::num::NonZeroUsize;

use sentinel_model::{
    AttributeKey, AttributeValue, Event, EventKind, ProducerEvent, SchemaVersion,
    SourceCapabilities, SourceCapability, SourceIdentity, SourceTier,
};

use crate::catalogue::exercise::{
    self, CounterpartMatches, SelectorExercise, SettledEvidence, TriggerExercise,
};
use crate::catalogue::{
    declaration, ParameterKind, PatternDeclaration, PatternId, RetirementPolicy, ABSENCE_AFTER,
    ABSENCE_BINDING_ATTRIBUTE, ABSENCE_UNTIL, ABSENCE_VETO, AGGREGATION_BINDING_ATTRIBUTE,
    AGGREGATION_LOOKBACK, AGGREGATION_MEMBER, AGGREGATION_THRESHOLD, AGGREGATION_TRIGGER,
    EXISTENCE_BINDING_ATTRIBUTE, EXISTENCE_CLOSE, EXISTENCE_OPEN, EXISTENCE_REQUIRED,
    PRECEDENCE_BINDING_ATTRIBUTE, PRECEDENCE_REQUIREMENT, PRECEDENCE_TRIGGER, TAINT_ACTION,
    TAINT_BINDING_ATTRIBUTE, TAINT_UNTRUSTED_CONTENT,
};
use crate::ir::{
    AbsenceIr, AggregateIr, ExistenceIr, MonitorIr, PrecedenceIr, RetirementIr, SelectorIr, TaintIr,
};
use crate::width::FilterWidthBudget;
use crate::{EventDependency, FilterIdentity, FilterSource};

/// The producer-contract facts an instantiation needs and no Pattern declares.
///
/// One field today. It is a named type rather than a bare argument so that the
/// answer to "where does the `SchemaVersion` come from?" is visible in the
/// signature instead of buried in a parameter list — see the module docs.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ProducerContract {
    schema_version: SchemaVersion,
}

impl ProducerContract {
    /// Declares the producer schema contract a Specification is written against.
    #[must_use]
    pub const fn new(schema_version: SchemaVersion) -> Self {
        Self { schema_version }
    }

    /// Returns the producer schema version.
    #[must_use]
    pub const fn schema_version(&self) -> &SchemaVersion {
        &self.schema_version
    }
}

/// A bound value for a [`ParameterKind::Selector`] parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SelectorValue {
    kind: EventKind,
    required_attributes: BTreeMap<AttributeKey, AttributeValue>,
}

impl SelectorValue {
    /// Binds a selector to an Event kind and the attribute valuations it requires.
    #[must_use]
    pub const fn new(
        kind: EventKind,
        required_attributes: BTreeMap<AttributeKey, AttributeValue>,
    ) -> Self {
        Self {
            kind,
            required_attributes,
        }
    }

    /// Returns the selected Event kind.
    #[must_use]
    pub const fn kind(&self) -> &EventKind {
        &self.kind
    }

    /// Returns the attribute valuations a matching Event must carry.
    #[must_use]
    pub const fn required_attributes(&self) -> &BTreeMap<AttributeKey, AttributeValue> {
        &self.required_attributes
    }

    /// Returns whether the producer matches kind and every required attribute.
    ///
    /// Delegates to the same helper [`crate::ir::SelectorIr::matches_producer`]
    /// uses. Candidate adversarial mutation must call this rather than copy
    /// the predicate (ADR-0027).
    #[must_use]
    pub fn matches_producer(&self, producer: &ProducerEvent) -> bool {
        crate::ir::producer_satisfies_selector(&self.kind, &self.required_attributes, producer)
    }

    /// Returns whether the Event matches kind and every required attribute.
    #[must_use]
    pub fn matches(&self, event: &Event) -> bool {
        self.matches_producer(&event.producer())
    }
}

/// A value bound to one declared Pattern parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ParameterValue {
    /// An Event attribute key.
    AttributeKey(AttributeKey),
    /// An Event selector.
    Selector(SelectorValue),
    /// A sequence-position lookback bound (not wall-clock duration).
    SequenceLookbackBound(u64),
    /// A non-negative integer count threshold.
    CountThreshold(u64),
}

impl ParameterValue {
    /// Returns the [`ParameterKind`] this value satisfies.
    #[must_use]
    pub const fn kind(&self) -> ParameterKind {
        match self {
            Self::AttributeKey(_) => ParameterKind::AttributeKey,
            Self::Selector(_) => ParameterKind::Selector,
            Self::SequenceLookbackBound(_) => ParameterKind::SequenceLookbackBound,
            Self::CountThreshold(_) => ParameterKind::CountThreshold,
        }
    }
}

/// A source bound to one declared evidence role, with where it is deployed.
///
/// `placement` and `capabilities` are what the **deployment** asserts about the
/// source; the Pattern's `EvidenceRequirement` is what the **Specification**
/// demands. Comparing them is [`PatternInstance::validate`]'s whole job.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct EvidenceBinding {
    source: SourceIdentity,
    placement: SourceTier,
    capabilities: SourceCapabilities,
}

impl EvidenceBinding {
    /// Binds an evidence role to a source as the deployment places it.
    #[must_use]
    pub const fn new(
        source: SourceIdentity,
        placement: SourceTier,
        capabilities: SourceCapabilities,
    ) -> Self {
        Self {
            source,
            placement,
            capabilities,
        }
    }

    /// Returns the bound source identity.
    #[must_use]
    pub const fn source(&self) -> &SourceIdentity {
        &self.source
    }

    /// Returns the assurance tier the deployment places this source at.
    #[must_use]
    pub const fn placement(&self) -> SourceTier {
        self.placement
    }

    /// Returns the capabilities the deployment asserts for this source.
    #[must_use]
    pub const fn capabilities(&self) -> &SourceCapabilities {
        &self.capabilities
    }
}

/// A refusal to instantiate: the Pattern's meaning would be undefined.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InstanceError {
    /// A declared parameter was not bound.
    MissingParameter {
        /// The Pattern being instantiated.
        pattern: PatternId,
        /// The unbound parameter's name.
        parameter: &'static str,
    },
    /// An argument was supplied that the Pattern does not declare.
    UnknownParameter {
        /// The Pattern being instantiated.
        pattern: PatternId,
        /// The undeclared argument's name.
        parameter: String,
    },
    /// A parameter was bound to a value of the wrong kind.
    ParameterKindMismatch {
        /// The Pattern being instantiated.
        pattern: PatternId,
        /// The parameter's name.
        parameter: &'static str,
        /// The kind the Pattern declares.
        expected: ParameterKind,
        /// The kind the supplied value has.
        actual: ParameterKind,
    },
    /// A declared evidence role was not bound to a source.
    MissingEvidenceBinding {
        /// The Pattern being instantiated.
        pattern: PatternId,
        /// The unbound role's name.
        role: &'static str,
    },
    /// A source was bound to a role the Pattern does not declare.
    UnknownEvidenceRole {
        /// The Pattern being instantiated.
        pattern: PatternId,
        /// The undeclared role's name.
        role: String,
    },
    /// The identity the Specification would be promoted under is incomplete.
    BlankIdentity {
        /// The Pattern being instantiated.
        pattern: PatternId,
        /// Which half of the identity is empty or whitespace-only.
        field: &'static str,
    },
}

impl Display for InstanceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingParameter { pattern, parameter } => {
                write!(formatter, "{pattern}: parameter {parameter} is not bound")
            }
            Self::UnknownParameter { pattern, parameter } => write!(
                formatter,
                "{pattern}: no parameter named {parameter} is declared"
            ),
            Self::ParameterKindMismatch {
                pattern,
                parameter,
                expected,
                actual,
            } => write!(
                formatter,
                "{pattern}: parameter {parameter} expects a {} but a {} was bound",
                expected.as_str(),
                actual.as_str()
            ),
            Self::MissingEvidenceBinding { pattern, role } => write!(
                formatter,
                "{pattern}: evidence role {role} is not bound to a source"
            ),
            Self::UnknownEvidenceRole { pattern, role } => write!(
                formatter,
                "{pattern}: no evidence role named {role} is declared"
            ),
            Self::BlankIdentity { pattern, field } => write!(
                formatter,
                "{pattern}: {field} is empty or whitespace-only, and every Verdict would carry \
                 it in its Assurance Envelope"
            ),
        }
    }
}

impl Error for InstanceError {}

/// Something an instantiation should know about its own conclusions.
///
/// Warnings never block instantiation. Each one says the Specification is
/// well-formed but that a deployment fact limits what its Verdicts can settle.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InstanceWarning {
    /// A source is placed below the tier its evidence role demands.
    ///
    /// The Specification is still well formed and still evaluates. What it
    /// cannot do is settle an authoritative Violate on this role's absence: the
    /// precedence monitor's anchor downgrade demotes such a Violate to
    /// `Inconclusive(prefix)` when no anchor-tier candidate exists. Placement is
    /// a deployment fact (ADR-0022), so the honest response is to say so, not to
    /// refuse.
    TierShortfall {
        /// The evidence role whose requirement is unmet.
        role: &'static str,
        /// The source bound to it.
        source: SourceIdentity,
        /// The tier the Specification demands.
        required: SourceTier,
        /// The tier the deployment places the source at.
        placement: SourceTier,
    },
    /// A source does not assert a capability its evidence role depends on.
    ///
    /// Sibling of [`InstanceWarning::TierShortfall`] and for the same reason:
    /// capabilities are asserted by the deployment's evidence, not chosen by the
    /// Pattern. A missing `CausalEdges` does not make the Specification
    /// ill-formed; it makes its reachability conclusions unreachable.
    CapabilityShortfall {
        /// The evidence role whose requirement is unmet.
        role: &'static str,
        /// The source bound to it.
        source: SourceIdentity,
        /// The capability the Specification depends on.
        capability: SourceCapability,
    },
    /// The evidence held no Event, so validating against it established nothing.
    ///
    /// Distinct from every warning below on purpose. "Nothing matched, and I
    /// examined 0 Events" is a finding about the validation; "nothing matched,
    /// and I examined 122 Events" is a finding about the Specification, and
    /// rendering the first as the second is the same overstatement issue #144 is
    /// about.
    NoEvidenceExamined,
    /// The obligation-raising selector names an Event kind the evidence never
    /// carried.
    ///
    /// **Issue #144's headline case.** The Specification is well-formed and
    /// evaluates cleanly; it just measures nothing, so every aggregation over
    /// its empty Verdict vector reads as a pass. A warning rather than an error
    /// for ADR-0022's reason, the same one [`InstanceWarning::TierShortfall`]
    /// carries: whether an Event kind is ever emitted is a fact about a
    /// deployment, not about the Specification, and refusing to instantiate
    /// would be this crate overruling a deployment it cannot see.
    ///
    /// One Trace's worth of this is **not** a defect on its own — a precedence
    /// obligation that never arose was never violated. See
    /// [`CorpusExercise`](crate::catalogue::CorpusExercise) for the fold that
    /// does separate the two.
    TriggerKindNeverOccurred {
        /// The declared parameter that bound the selector.
        parameter: &'static str,
        /// The Event kind it names.
        kind: EventKind,
        /// How many Events were examined to reach that answer.
        events_examined: NonZeroUsize,
    },
    /// The obligation-raising selector's Event kind occurs, but no occurrence
    /// satisfied its required attribute valuation.
    ///
    /// Split from [`InstanceWarning::TriggerKindNeverOccurred`] because the two
    /// name different authoring mistakes with different remediations: fix the
    /// kind, or fix the valuation.
    TriggerValuationNeverMatched {
        /// The declared parameter that bound the selector.
        parameter: &'static str,
        /// The Event kind it names.
        kind: EventKind,
        /// How many Events carried the kind without matching the valuation.
        kind_matches: NonZeroUsize,
    },
    /// Events matched the obligation-raising selector, and none named a Binding.
    ///
    /// No Slice exists to hold a Verdict, so no Verdict can be produced. This is
    /// the Binding-attribute half of the vacuous instantiation, which
    /// `core/tests/catalogue_grounding.rs` previously had to catch after
    /// evaluation because `sentinel-compile` could not see the evidence.
    TriggerRaisedNoObligation {
        /// The declared parameter that bound the selector.
        parameter: &'static str,
        /// The Binding attribute none of the matching Events carried.
        binding_attribute: String,
        /// How many Events matched the selector.
        matches: NonZeroUsize,
    },
}

impl Display for InstanceWarning {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::TierShortfall {
                role,
                source,
                required,
                placement,
            } => write!(
                formatter,
                "role {role} needs a {required:?}-tier source but {}/{} is placed {placement:?}; \
                 conclusions resting on this role's absence cannot settle authoritatively",
                source.monitored_application(),
                source.source()
            ),
            Self::CapabilityShortfall {
                role,
                source,
                capability,
            } => write!(
                formatter,
                "role {role} depends on {capability:?} but {}/{} does not assert it",
                source.monitored_application(),
                source.source()
            ),
            Self::NoEvidenceExamined => write!(
                formatter,
                "no Event was examined, so this validation established nothing about what \
                 the Specification measures"
            ),
            Self::TriggerKindNeverOccurred {
                parameter,
                kind,
                events_examined,
            } => write!(
                formatter,
                "parameter {parameter} selects Event kind {kind}, which none of the \
                 {events_examined} Events examined carried; this Specification raised no \
                 obligation and so measured nothing"
            ),
            Self::TriggerValuationNeverMatched {
                parameter,
                kind,
                kind_matches,
            } => write!(
                formatter,
                "parameter {parameter} selects Event kind {kind}, which {kind_matches} Events \
                 carried, none of them satisfying its required attribute valuation; this \
                 Specification raised no obligation and so measured nothing"
            ),
            Self::TriggerRaisedNoObligation {
                parameter,
                binding_attribute,
                matches,
            } => write!(
                formatter,
                "parameter {parameter} matched {matches} Events, none of which names a Binding \
                 under {binding_attribute}, so no Slice exists to hold a Verdict"
            ),
        }
    }
}

/// One Pattern with its parameters and evidence roles bound.
///
/// Construct with [`instantiate`]. Implements [`FilterSource`], so a set of
/// instances compiles straight into a Probe-shippable Event Filter.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct PatternInstance {
    pattern: PatternId,
    identity: FilterIdentity,
    contract: ProducerContract,
    arguments: BTreeMap<String, ParameterValue>,
    evidence: BTreeMap<String, EvidenceBinding>,
    ir: MonitorIr,
}

impl PatternInstance {
    /// Returns the Pattern this instance came from.
    #[must_use]
    pub const fn pattern(&self) -> PatternId {
        self.pattern
    }

    /// Returns the declaration this instance was built against.
    #[must_use]
    pub const fn declaration(&self) -> &'static PatternDeclaration {
        declaration(self.pattern)
    }

    /// Returns the identity this Specification is promoted under.
    #[must_use]
    pub const fn identity(&self) -> &FilterIdentity {
        &self.identity
    }

    /// Returns the producer schema contract the selectors are written against.
    #[must_use]
    pub const fn contract(&self) -> &ProducerContract {
        &self.contract
    }

    /// Returns the monitor IR the Verification Core lowers.
    #[must_use]
    pub const fn ir(&self) -> &MonitorIr {
        &self.ir
    }

    /// Returns the value bound to one declared parameter, if it is declared.
    #[must_use]
    pub fn argument(&self, parameter: &str) -> Option<&ParameterValue> {
        self.arguments.get(parameter)
    }

    /// Returns the source bound to one declared evidence role, if it is declared.
    #[must_use]
    pub fn evidence_binding(&self, role: &str) -> Option<&EvidenceBinding> {
        self.evidence.get(role)
    }

    /// Reports every way the deployment falls short of what this Pattern's
    /// conclusions need.
    ///
    /// Returns warnings in declared-role order, so the result is deterministic
    /// and diffable. An empty vector means every role is bound to a source the
    /// deployment places at or above the demanded tier, holding every capability
    /// the role depends on.
    #[must_use]
    pub fn validate(&self) -> Vec<InstanceWarning> {
        let mut warnings = Vec::new();
        for requirement in self.declaration().evidence {
            let Some(binding) = self.evidence.get(requirement.role) else {
                continue; // Unreachable: instantiate() rejects an unbound role.
            };
            if requirement.minimum_tier == SourceTier::Anchor
                && binding.placement == SourceTier::Contributing
            {
                warnings.push(InstanceWarning::TierShortfall {
                    role: requirement.role,
                    source: binding.source.clone(),
                    required: requirement.minimum_tier,
                    placement: binding.placement,
                });
            }
            for capability in requirement.capabilities {
                if !binding.capabilities.contains(*capability) {
                    warnings.push(InstanceWarning::CapabilityShortfall {
                        role: requirement.role,
                        source: binding.source.clone(),
                        capability: *capability,
                    });
                }
            }
        }
        warnings
    }

    /// Censuses what this instance's selectors actually matched.
    ///
    /// The question issue #144 needs answered before a Verdict can exist: *did
    /// this Specification raise any obligation at all?* Read
    /// [`exercise`](crate::catalogue::SettledEvidence)'s module for why the
    /// answer is a census rather than a judgement, and why the parameter type
    /// carries the settled-evidence assertion.
    ///
    /// The `match` is exhaustive over [`PatternId`], which is the whole defence
    /// against a Pattern whose second selector must never match: for CAT-4 the
    /// untrusted-content selector is the one that must not reach an action, and
    /// censusing *it* would report a clean deployment as never exercised.
    /// Adding such a Pattern is a compile error here rather than a silent
    /// inheritance of a precedence-shaped judgement, and the author has to name
    /// the obligation-raising selector explicitly.
    #[must_use]
    pub fn exercise(&self, evidence: SettledEvidence<'_>) -> SelectorExercise {
        match self.pattern {
            PatternId::PrecedenceApproval => self.exercise_precedence(evidence),
            PatternId::ProvenanceTaint => self.exercise_taint(evidence),
            PatternId::AbsenceAfterUntil => self.exercise_absence(evidence),
            PatternId::AggregationCountWithin => self.exercise_aggregate(evidence),
            PatternId::ExistenceBeforeScopeClose => self.exercise_existence(evidence),
        }
    }

    /// Reports every way this instance falls short, including what it measured.
    ///
    /// [`PatternInstance::validate`]'s warnings followed by the census's, in
    /// that order, both derived from one pass so the two cannot disagree.
    ///
    /// Separate from [`PatternInstance::validate`] rather than replacing it:
    /// tier and capability shortfalls are answerable from the instantiation
    /// alone, and a caller with no evidence to hand must still be able to ask
    /// for them. This is the seam #21 and #35 call, because a generated
    /// Candidate is exactly the input that produces a selector naming something
    /// nobody emits.
    #[must_use]
    pub fn validate_against(&self, evidence: SettledEvidence<'_>) -> Vec<InstanceWarning> {
        let mut warnings = self.validate();
        warnings.extend(self.exercise_warnings(self.exercise(evidence)));
        warnings
    }

    fn exercise_precedence(&self, evidence: SettledEvidence<'_>) -> SelectorExercise {
        let ir = self.precedence_ir();
        let mut kind_matches = 0_usize;
        let mut matches = 0_usize;
        let mut obligations = 0_usize;
        let mut requirement_matches = 0_usize;
        let mut binding_carriers = 0_usize;

        for event in evidence.events() {
            let binds = ir.binding_of(event).is_some();
            if binds {
                binding_carriers += 1;
            }
            if ir.requirement.matches(event) {
                requirement_matches += 1;
            }
            if !ir.trigger.matches_kind(event) {
                continue;
            }
            kind_matches += 1;
            if !ir.trigger.matches(event) {
                continue;
            }
            matches += 1;
            // The conjunction, not two independent totals: the Verification Core
            // raises an obligation only where one Event both matches the trigger
            // and names a Binding.
            if binds {
                obligations += 1;
            }
        }

        SelectorExercise {
            trigger: exercise::classify(evidence.examined(), kind_matches, matches, obligations),
            counterpart: CounterpartMatches::Discharging(requirement_matches),
            binding_carriers,
            events_examined: evidence.examined(),
        }
    }

    /// Censuses CAT-4 on its **action** selector, never on its taint marker.
    ///
    /// The trap `exercise.rs` names, in its concrete form. An evidence set can
    /// be full of untrusted-content markers and raise no obligation at all,
    /// because no action occurred for the property to be about; and a clean
    /// deployment emits no marker whatsoever while raising an obligation per
    /// action. Keying the census on the marker would invert both readings.
    ///
    /// Otherwise identical in structure to
    /// [`PatternInstance::exercise_precedence`], including the conjunction: an
    /// obligation exists only where one Event both matches the action selector
    /// and names a Binding.
    fn exercise_taint(&self, evidence: SettledEvidence<'_>) -> SelectorExercise {
        let ir = self.taint_ir();
        let mut kind_matches = 0_usize;
        let mut matches = 0_usize;
        let mut obligations = 0_usize;
        let mut untrusted_matches = 0_usize;
        let mut binding_carriers = 0_usize;

        for event in evidence.events() {
            let binds = ir.binding_of(event).is_some();
            if binds {
                binding_carriers += 1;
            }
            if ir.untrusted_content.matches(event) {
                untrusted_matches += 1;
            }
            if !ir.action.matches_kind(event) {
                continue;
            }
            kind_matches += 1;
            if !ir.action.matches(event) {
                continue;
            }
            matches += 1;
            if binds {
                obligations += 1;
            }
        }

        SelectorExercise {
            trigger: exercise::classify(evidence.examined(), kind_matches, matches, obligations),
            counterpart: CounterpartMatches::Falsifying(untrusted_matches),
            binding_carriers,
            events_examined: evidence.examined(),
        }
    }

    /// Censuses CAT-2 on its **until** selector, never on its veto.
    ///
    /// The absence-shaped form of the trap `exercise.rs` names. An evidence set
    /// can be full of veto Events and raise no obligation at all, because no
    /// `until` closed an interval for the property to be about; and a clean
    /// deployment emits no veto whatsoever while raising an obligation per
    /// `until`. Keying the census on the veto would invert both readings.
    ///
    /// The counterpart is [`CounterpartMatches::Interval`]: `opening` counts
    /// `after` matches and `falsifying` counts `veto` matches. Promotion polarity
    /// still reads the falsifying half.
    fn exercise_absence(&self, evidence: SettledEvidence<'_>) -> SelectorExercise {
        let ir = self.absence_ir();
        let mut kind_matches = 0_usize;
        let mut matches = 0_usize;
        let mut obligations = 0_usize;
        let mut after_matches = 0_usize;
        let mut veto_matches = 0_usize;
        let mut binding_carriers = 0_usize;

        for event in evidence.events() {
            let binds = ir.binding_of(event).is_some();
            if binds {
                binding_carriers += 1;
            }
            if ir.after.matches(event) {
                after_matches += 1;
            }
            if ir.veto.matches(event) {
                veto_matches += 1;
            }
            if !ir.until.matches_kind(event) {
                continue;
            }
            kind_matches += 1;
            if !ir.until.matches(event) {
                continue;
            }
            matches += 1;
            if binds {
                obligations += 1;
            }
        }

        SelectorExercise {
            trigger: exercise::classify(evidence.examined(), kind_matches, matches, obligations),
            counterpart: CounterpartMatches::Interval {
                opening: after_matches,
                falsifying: veto_matches,
            },
            binding_carriers,
            events_examined: evidence.examined(),
        }
    }

    /// Censuses CAT-5 on its **trigger** selector, never on its member.
    ///
    /// The aggregation-shaped form of the trap `exercise.rs` names. An evidence
    /// set can be full of counted member Events and raise no obligation at all,
    /// because no trigger assessed the cap; and a clean under-threshold
    /// deployment may emit no member whatsoever while raising an obligation per
    /// trigger. Keying the census on the member would invert both readings.
    ///
    /// The counterpart is [`CounterpartMatches::Falsifying`]: member matches are
    /// the reported count population a cold over-threshold `Fails` would draw
    /// from once Coverage closes — reported rather than judged here.
    fn exercise_aggregate(&self, evidence: SettledEvidence<'_>) -> SelectorExercise {
        let ir = self.aggregate_ir();
        let mut kind_matches = 0_usize;
        let mut matches = 0_usize;
        let mut obligations = 0_usize;
        let mut member_matches = 0_usize;
        let mut binding_carriers = 0_usize;

        for event in evidence.events() {
            let binds = ir.binding_of(event).is_some();
            if binds {
                binding_carriers += 1;
            }
            if ir.member.matches(event) {
                member_matches += 1;
            }
            if !ir.trigger.matches_kind(event) {
                continue;
            }
            kind_matches += 1;
            if !ir.trigger.matches(event) {
                continue;
            }
            matches += 1;
            if binds {
                obligations += 1;
            }
        }

        SelectorExercise {
            trigger: exercise::classify(evidence.examined(), kind_matches, matches, obligations),
            counterpart: CounterpartMatches::Falsifying(member_matches),
            binding_carriers,
            events_examined: evidence.examined(),
        }
    }

    /// Censuses CAT-3 on its **open** selector, never on its required.
    ///
    /// A Trace full of compensations (required matches) with no saga open raises
    /// no obligation; keying the census on `required` would report that Trace as
    /// exercised and invent a vacuous pass surface. The counterpart is
    /// [`CounterpartMatches::Discharging`]: required matches discharge the
    /// existence obligation when they fall inside `(open, close]`.
    fn exercise_existence(&self, evidence: SettledEvidence<'_>) -> SelectorExercise {
        let ir = self.existence_ir();
        let mut kind_matches = 0_usize;
        let mut matches = 0_usize;
        let mut obligations = 0_usize;
        let mut required_matches = 0_usize;
        let mut binding_carriers = 0_usize;

        for event in evidence.events() {
            let binds = ir.binding_of(event).is_some();
            if binds {
                binding_carriers += 1;
            }
            if ir.required.matches(event) {
                required_matches += 1;
            }
            if !ir.open.matches_kind(event) {
                continue;
            }
            kind_matches += 1;
            if !ir.open.matches(event) {
                continue;
            }
            matches += 1;
            if binds {
                obligations += 1;
            }
        }

        SelectorExercise {
            trigger: exercise::classify(evidence.examined(), kind_matches, matches, obligations),
            counterpart: CounterpartMatches::Discharging(required_matches),
            binding_carriers,
            events_examined: evidence.examined(),
        }
    }

    /// Turns one census into the warnings that census licenses.
    ///
    /// Flat variants rather than one variant carrying a [`TriggerExercise`], so
    /// no warning can hold a census value that contradicts it.
    fn exercise_warnings(&self, exercise: SelectorExercise) -> Vec<InstanceWarning> {
        let (parameter, selector, binding_attribute) = match self.pattern {
            PatternId::PrecedenceApproval => {
                let ir = self.precedence_ir();
                (PRECEDENCE_TRIGGER, &ir.trigger, &ir.binding_attribute)
            }
            PatternId::ProvenanceTaint => {
                let ir = self.taint_ir();
                (TAINT_ACTION, &ir.action, &ir.binding_attribute)
            }
            PatternId::AbsenceAfterUntil => {
                let ir = self.absence_ir();
                (ABSENCE_UNTIL, &ir.until, &ir.binding_attribute)
            }
            PatternId::AggregationCountWithin => {
                let ir = self.aggregate_ir();
                (AGGREGATION_TRIGGER, &ir.trigger, &ir.binding_attribute)
            }
            PatternId::ExistenceBeforeScopeClose => {
                let ir = self.existence_ir();
                (EXISTENCE_OPEN, &ir.open, &ir.binding_attribute)
            }
        };
        Self::warnings_for(parameter, selector, binding_attribute, exercise)
    }

    /// Turns the census of one obligation-raising selector into its warnings.
    ///
    /// Shared across Patterns because the four answers mean the same thing
    /// whichever selector raised the obligation — *the kind is never emitted*,
    /// *the valuation never holds*, *nothing names a Binding*, *something was
    /// measured*. What is **not** shared is which selector that is, which is why
    /// the caller resolves it in an exhaustive `match` and passes it in.
    fn warnings_for(
        parameter: &'static str,
        selector: &SelectorIr,
        binding_attribute: &str,
        exercise: SelectorExercise,
    ) -> Vec<InstanceWarning> {
        match exercise.trigger {
            TriggerExercise::NoEvidenceExamined => vec![InstanceWarning::NoEvidenceExamined],
            TriggerExercise::KindNeverOccurred { events_examined } => {
                vec![InstanceWarning::TriggerKindNeverOccurred {
                    parameter,
                    kind: selector.kind.clone(),
                    events_examined,
                }]
            }
            TriggerExercise::ValuationNeverMatched { kind_matches } => {
                vec![InstanceWarning::TriggerValuationNeverMatched {
                    parameter,
                    kind: selector.kind.clone(),
                    kind_matches,
                }]
            }
            TriggerExercise::MatchedWithoutBinding { matches } => {
                vec![InstanceWarning::TriggerRaisedNoObligation {
                    parameter,
                    binding_attribute: binding_attribute.to_owned(),
                    matches,
                }]
            }
            TriggerExercise::RaisedObligations { .. } => Vec::new(),
        }
    }

    /// Reads the precedence monitor this instance lowered to.
    ///
    /// Reachable only from a [`PatternId::PrecedenceApproval`] arm, and
    /// `lower` is the only producer of a [`MonitorIr`], so the pairing cannot
    /// come apart from outside this module.
    /// `every_pattern_lowers_to_the_monitor_its_family_needs` is the test that
    /// fails if `lower` is edited so that it does.
    fn precedence_ir(&self) -> &PrecedenceIr {
        self.ir
            .as_precedence()
            .expect("CAT-1 lowers to MonitorIr::Precedence")
    }

    /// Reads the taint monitor this instance lowered to.
    ///
    /// [`PatternInstance::precedence_ir`]'s sibling, with the same guard.
    fn taint_ir(&self) -> &TaintIr {
        self.ir
            .as_taint()
            .expect("CAT-4 lowers to MonitorIr::Taint")
    }

    /// Reads the absence monitor this instance lowered to.
    ///
    /// [`PatternInstance::precedence_ir`]'s sibling, with the same guard.
    fn absence_ir(&self) -> &AbsenceIr {
        self.ir
            .as_absence()
            .expect("CAT-2 lowers to MonitorIr::Absence")
    }

    /// Reads the aggregate monitor this instance lowered to.
    ///
    /// [`PatternInstance::precedence_ir`]'s sibling, with the same guard.
    fn aggregate_ir(&self) -> &AggregateIr {
        self.ir
            .as_aggregate()
            .expect("CAT-5 lowers to MonitorIr::Aggregate")
    }

    fn existence_ir(&self) -> &ExistenceIr {
        self.ir
            .as_existence()
            .expect("CAT-3 lowers to MonitorIr::Existence")
    }
}

/// Binds a Pattern's parameters and evidence roles into a [`PatternInstance`].
///
/// Every declared parameter must be bound to a value of the declared kind, and
/// every declared evidence role must be bound to a source. Nothing undeclared may
/// be supplied: an argument the Pattern does not read would be a value #21
/// generated that silently does nothing, which is exactly the failure mode a
/// declared grammar exists to prevent.
///
/// Tier and capability shortfalls are **not** errors. Call
/// [`PatternInstance::validate`] for those, and read this module's docs for why.
///
/// # Errors
///
/// Returns [`InstanceError::MissingParameter`] or
/// [`InstanceError::MissingEvidenceBinding`] when something declared is unbound,
/// [`InstanceError::UnknownParameter`] or [`InstanceError::UnknownEvidenceRole`]
/// when something undeclared is supplied,
/// [`InstanceError::ParameterKindMismatch`] when a bound value has the wrong
/// kind, and [`InstanceError::BlankIdentity`] when Specification identity is
/// empty or whitespace-only. Blank attribute-key / selector-key names are
/// unrepresentable in [`ParameterValue`] / [`SelectorValue`] (issue #203) —
/// see this module's docs for where value checks stop and why.
pub fn instantiate(
    pattern: PatternId,
    identity: FilterIdentity,
    contract: ProducerContract,
    arguments: BTreeMap<String, ParameterValue>,
    evidence: BTreeMap<String, EvidenceBinding>,
) -> Result<PatternInstance, InstanceError> {
    let declared = declaration(pattern);

    // Checked here rather than left to compile_filter, because both halves of
    // the identity are copied verbatim into every Verdict's AssuranceEnvelope
    // (core/src/verdict.rs) and nothing downstream of the Core compares them to
    // anything. A blank one is an assurance artifact that names no
    // Specification, and only the Filter path would ever have refused it.
    // Identity fields stay public `String` (#21), so BlankIdentity remains —
    // unlike attribute keys, which `AttributeKey` seals at the type level.
    for (field, value) in [
        ("specification_id", identity.specification_id.as_str()),
        (
            "specification_version",
            identity.specification_version.as_str(),
        ),
    ] {
        if is_blank(value) {
            return Err(InstanceError::BlankIdentity { pattern, field });
        }
    }

    for parameter in declared.parameters {
        let Some(value) = arguments.get(parameter.name) else {
            return Err(InstanceError::MissingParameter {
                pattern,
                parameter: parameter.name,
            });
        };
        if value.kind() != parameter.kind {
            return Err(InstanceError::ParameterKindMismatch {
                pattern,
                parameter: parameter.name,
                expected: parameter.kind,
                actual: value.kind(),
            });
        }
    }
    for supplied in arguments.keys() {
        if declared.parameter(supplied).is_none() {
            return Err(InstanceError::UnknownParameter {
                pattern,
                parameter: supplied.clone(),
            });
        }
    }

    for requirement in declared.evidence {
        if !evidence.contains_key(requirement.role) {
            return Err(InstanceError::MissingEvidenceBinding {
                pattern,
                role: requirement.role,
            });
        }
    }
    for supplied in evidence.keys() {
        if declared.requirement(supplied).is_none() {
            return Err(InstanceError::UnknownEvidenceRole {
                pattern,
                role: supplied.clone(),
            });
        }
    }

    let ir = lower(
        pattern, &identity, &contract, &arguments, &evidence, declared,
    );
    Ok(PatternInstance {
        pattern,
        identity,
        contract,
        arguments,
        evidence,
        ir,
    })
}

/// Returns whether a producer-supplied string carries no content.
///
/// The same rule `sentinel_model::identifiers` apply to typed identifiers,
/// restated for Specification identity fields that remain public `String`.
fn is_blank(value: &str) -> bool {
    value.trim().is_empty()
}

/// Builds the monitor IR for one validated set of bindings.
///
/// The `match` is exhaustive over [`PatternId`], so a Pattern added to the
/// catalogue without a lowering is a compile error rather than a runtime
/// surprise. That is the whole benefit Q3 traded parameter type-safety for.
///
/// `retirement` is read off the Pattern's own declaration through
/// [`retirement`] rather than written per family, so a Pattern that changes its
/// retirement policy changes what the Core sees without a second edit here.
fn lower(
    pattern: PatternId,
    identity: &FilterIdentity,
    contract: &ProducerContract,
    arguments: &BTreeMap<String, ParameterValue>,
    evidence: &BTreeMap<String, EvidenceBinding>,
    declared: &'static PatternDeclaration,
) -> MonitorIr {
    match pattern {
        PatternId::PrecedenceApproval => MonitorIr::Precedence(PrecedenceIr {
            specification_id: identity.specification_id.clone(),
            specification_version: identity.specification_version.clone(),
            schema_version: contract.schema_version().clone(),
            binding_attribute: attribute_key(arguments, PRECEDENCE_BINDING_ATTRIBUTE)
                .as_str()
                .to_owned(),
            trigger: selector(arguments, PRECEDENCE_TRIGGER),
            requirement: selector(arguments, PRECEDENCE_REQUIREMENT),
            required_sources: required_sources(evidence, declared),
            retirement: retirement(declared),
        }),
        PatternId::ProvenanceTaint => MonitorIr::Taint(TaintIr {
            specification_id: identity.specification_id.clone(),
            specification_version: identity.specification_version.clone(),
            schema_version: contract.schema_version().clone(),
            binding_attribute: attribute_key(arguments, TAINT_BINDING_ATTRIBUTE)
                .as_str()
                .to_owned(),
            action: selector(arguments, TAINT_ACTION),
            untrusted_content: selector(arguments, TAINT_UNTRUSTED_CONTENT),
            required_sources: required_sources(evidence, declared),
            retirement: retirement(declared),
        }),
        PatternId::AbsenceAfterUntil => MonitorIr::Absence(AbsenceIr {
            specification_id: identity.specification_id.clone(),
            specification_version: identity.specification_version.clone(),
            schema_version: contract.schema_version().clone(),
            binding_attribute: attribute_key(arguments, ABSENCE_BINDING_ATTRIBUTE)
                .as_str()
                .to_owned(),
            after: selector(arguments, ABSENCE_AFTER),
            until: selector(arguments, ABSENCE_UNTIL),
            veto: selector(arguments, ABSENCE_VETO),
            required_sources: required_sources(evidence, declared),
            retirement: retirement(declared),
        }),
        PatternId::AggregationCountWithin => MonitorIr::Aggregate(AggregateIr {
            specification_id: identity.specification_id.clone(),
            specification_version: identity.specification_version.clone(),
            schema_version: contract.schema_version().clone(),
            binding_attribute: attribute_key(arguments, AGGREGATION_BINDING_ATTRIBUTE)
                .as_str()
                .to_owned(),
            trigger: selector(arguments, AGGREGATION_TRIGGER),
            member: selector(arguments, AGGREGATION_MEMBER),
            lookback_sequences: sequence_lookback(arguments, AGGREGATION_LOOKBACK),
            threshold: count_threshold(arguments, AGGREGATION_THRESHOLD),
            required_sources: required_sources(evidence, declared),
            retirement: retirement(declared),
        }),
        PatternId::ExistenceBeforeScopeClose => MonitorIr::Existence(ExistenceIr {
            specification_id: identity.specification_id.clone(),
            specification_version: identity.specification_version.clone(),
            schema_version: contract.schema_version().clone(),
            binding_attribute: attribute_key(arguments, EXISTENCE_BINDING_ATTRIBUTE)
                .as_str()
                .to_owned(),
            open: selector(arguments, EXISTENCE_OPEN),
            required: selector(arguments, EXISTENCE_REQUIRED),
            close: selector(arguments, EXISTENCE_CLOSE),
            required_sources: required_sources(evidence, declared),
            retirement: retirement(declared),
        }),
    }
}

/// Lowers the Pattern's declared retirement policy into monitor IR.
///
/// Total over [`RetirementPolicy`], so a policy variant added to the catalogue
/// without an IR dual is a compile error. The two enums are deliberately
/// distinct types — see [`RetirementIr`] — and this is the one place they meet.
const fn retirement(declared: &'static PatternDeclaration) -> RetirementIr {
    match declared.retirement {
        RetirementPolicy::NotRequired => RetirementIr::NotRequired,
        RetirementPolicy::Deadline => RetirementIr::Deadline,
        RetirementPolicy::Inactivity => RetirementIr::Inactivity,
    }
}

/// Reads an attribute-key argument. Infallible after `instantiate`'s checks.
fn attribute_key(arguments: &BTreeMap<String, ParameterValue>, name: &str) -> AttributeKey {
    match arguments.get(name) {
        Some(ParameterValue::AttributeKey(key)) => key.clone(),
        _ => unreachable!("instantiate validated {name} as an attribute key"),
    }
}

/// Reads a selector argument. Infallible after `instantiate`'s checks.
fn selector(arguments: &BTreeMap<String, ParameterValue>, name: &str) -> SelectorIr {
    match arguments.get(name) {
        Some(ParameterValue::Selector(value)) => SelectorIr {
            kind: value.kind().clone(),
            required_attributes: value.required_attributes().clone(),
        },
        _ => unreachable!("instantiate validated {name} as a selector"),
    }
}

/// Reads a sequence-lookback argument. Infallible after `instantiate`'s checks.
fn sequence_lookback(arguments: &BTreeMap<String, ParameterValue>, name: &str) -> u64 {
    match arguments.get(name) {
        Some(ParameterValue::SequenceLookbackBound(bound)) => *bound,
        _ => unreachable!("instantiate validated {name} as a sequence-lookback-bound"),
    }
}

/// Reads a count-threshold argument. Infallible after `instantiate`'s checks.
fn count_threshold(arguments: &BTreeMap<String, ParameterValue>, name: &str) -> u64 {
    match arguments.get(name) {
        Some(ParameterValue::CountThreshold(threshold)) => *threshold,
        _ => unreachable!("instantiate validated {name} as a count-threshold"),
    }
}

/// Collapses role bindings into the required-source set the Core reads.
///
/// The tier carried is the **declaration's** minimum, never the deployment's
/// placement: ADR-0022 makes the required tier a Specification property, and a
/// Specification whose demands shrank to match a weak deployment would be a
/// Specification that can never report the deployment is weak. Two roles binding
/// one source keep the stricter minimum.
fn required_sources(
    evidence: &BTreeMap<String, EvidenceBinding>,
    declared: &'static PatternDeclaration,
) -> BTreeMap<SourceIdentity, SourceTier> {
    let mut required: BTreeMap<SourceIdentity, SourceTier> = BTreeMap::new();
    for requirement in declared.evidence {
        let Some(binding) = evidence.get(requirement.role) else {
            continue; // Unreachable: instantiate() rejects an unbound role.
        };
        required
            .entry(binding.source.clone())
            .and_modify(|tier| {
                if requirement.minimum_tier == SourceTier::Anchor {
                    *tier = SourceTier::Anchor;
                }
            })
            .or_insert(requirement.minimum_tier);
    }
    required
}

/// The Pattern instance is the Filter source: this is the seam
/// `compile/src/filter.rs` reserved for #14.
///
/// The dependency set is derived from the IR's own selectors rather than
/// declared separately, which closes the trust boundary `filter.rs` names: "a
/// Specification that omits a dependency it actually reads is unsound and
/// undetectable here — that is a Pattern-authoring bug, tested per-Pattern in
/// #14." A Pattern cannot omit a selector it matches on, because the same value
/// is both what the monitor reads and what the Filter admits.
impl FilterSource for PatternInstance {
    fn filter_identity(&self) -> FilterIdentity {
        self.identity.clone()
    }

    fn dependencies(&self) -> Vec<EventDependency> {
        // Exhaustive per monitor family rather than "every selector the IR
        // holds", because which selectors exist is the family's own shape. A
        // family whose Filter must admit a third kind is a compile error here.
        match &self.ir {
            MonitorIr::Precedence(precedence) => vec![
                dependency(&precedence.trigger),
                dependency(&precedence.requirement),
            ],
            // The untrusted-content selector is a dependency even though a
            // matching Event is what falsifies the property: a Filter that
            // dropped it would leave the monitor unable to see the very Event
            // its Violate rests on, and every Verdict would read clean. This is
            // the trust boundary `filter.rs` names, in the direction where
            // getting it wrong inverts the answer rather than losing one.
            MonitorIr::Taint(taint) => vec![
                dependency(&taint.action),
                dependency(&taint.untrusted_content),
            ],
            // Same trust-boundary argument as CAT-4: dropping `after` or `veto`
            // would make the interval invisible or every Verdict read clean.
            MonitorIr::Absence(absence) => vec![
                dependency(&absence.after),
                dependency(&absence.until),
                dependency(&absence.veto),
            ],
            // Same trust-boundary argument: dropping `member` would leave the
            // cold path unable to count the Events its Fail rests on.
            MonitorIr::Aggregate(aggregate) => {
                vec![
                    dependency(&aggregate.trigger),
                    dependency(&aggregate.member),
                ]
            }
            MonitorIr::Existence(existence) => vec![
                dependency(&existence.open),
                dependency(&existence.required),
                dependency(&existence.close),
            ],
        }
    }

    fn binding_attribute_keys(&self) -> Vec<AttributeKey> {
        // Load-bearing. A trigger selector commonly requires no attributes, so
        // without surfacing the Binding attribute here the projection would drop
        // it and the Core's binding_of() would find nothing — a silent Verdict
        // inversion rather than a failure. Clone the typed parameter retained at
        // instantiate rather than re-parsing the public IR `String` (issue #203).
        // Every monitor family declares the same parameter name today; an arm
        // added without one is the failure this comment describes.
        match self.arguments.get(PRECEDENCE_BINDING_ATTRIBUTE) {
            Some(ParameterValue::AttributeKey(key)) => vec![key.clone()],
            _ => unreachable!("instantiate validated binding_attribute as an attribute key"),
        }
    }

    fn required_sources(&self) -> Vec<SourceIdentity> {
        self.ir.required_sources().keys().cloned().collect()
    }

    fn width_budget(&self) -> FilterWidthBudget {
        // Lifted to the template, which is what `compile/src/width.rs` asked #14
        // for: every instance of a Pattern reports against one declared budget.
        self.declaration().width.budget
    }
}

/// Projects one selector into the Event kind and attribute keys it reads.
fn dependency(selector: &SelectorIr) -> EventDependency {
    EventDependency {
        kind: selector.kind.clone(),
        attribute_keys: selector
            .required_attributes
            .keys()
            .cloned()
            .collect::<BTreeSet<AttributeKey>>(),
    }
}

#[cfg(test)]
mod attribute_key_close {
    use super::*;

    /// `BlankAttributeKey` / `BlankSelectorAttributeKey` must not reappear.
    ///
    /// Defined next to [`InstanceError`] because the enum is `#[non_exhaustive]`
    /// outside this crate — a defining-crate match without `_` is the pin.
    #[test]
    fn instance_error_has_no_blank_attribute_key_arms() {
        // Owned match keeps exhaustiveness over InstanceError without a wildcard —
        // that is the pin that BlankAttributeKey / BlankSelectorAttributeKey stay gone.
        #[allow(clippy::needless_pass_by_value)]
        fn assert_no_blank_attribute_arms(error: InstanceError) {
            match error {
                InstanceError::MissingParameter { .. }
                | InstanceError::UnknownParameter { .. }
                | InstanceError::ParameterKindMismatch { .. }
                | InstanceError::MissingEvidenceBinding { .. }
                | InstanceError::UnknownEvidenceRole { .. }
                | InstanceError::BlankIdentity { .. } => {}
            }
        }
        assert_no_blank_attribute_arms(InstanceError::BlankIdentity {
            pattern: PatternId::PrecedenceApproval,
            field: "specification_id",
        });
    }
}
