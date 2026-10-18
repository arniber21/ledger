//! Monitor IR: the only thing the Verification Core ever sees of a Pattern.
//!
//! `compile` cannot depend on `core` — `core/Cargo.toml` already depends on
//! `sentinel-compile`, so the reverse edge would be a cycle. A Pattern therefore
//! cannot construct a `PrecedenceSpecification` directly. This module is the
//! seam: [`PatternInstance`](crate::catalogue::PatternInstance) lowers to plain
//! data owned by `compile`, and `core` lowers *that* into its own types.
//!
//! What crosses the seam is deliberately narrow. The Core sees selectors, a
//! Binding attribute, required sources and a schema version. It sees no
//! `PatternId`, no display name, no width budget, no grounding reference and no
//! evidence grade — none of which a monitor could act on, and all of which would
//! make the Core a consumer of the catalogue rather than of one Specification.
//!
//! # Four variants, and the grow-vs-move decision
//!
//! #14a shipped [`MonitorIr`] with one variant and no `TryFrom` ceremony, on the
//! adversarial review's argument: do not build a general IR layer for one Pattern
//! family on a hypothesis about the second. CAT-4 `provenance.taint` landed the
//! second family; CAT-2 `absence.after-until` lands the third; CAT-5
//! `aggregation.count-within` lands the fourth.
//!
//! **Decision:** grow [`MonitorIr`] variants. Do not move Specification types
//! into `compile`.
//!
//! [`TaintIr`] is **not** [`PrecedenceIr`] with a flag. The two carry the same
//! four fields of Specification identity and the same required-source map, and
//! then diverge on the thing that matters: `PrecedenceIr`'s second selector
//! *discharges* an obligation and `TaintIr`'s *falsifies* one. Sharing a struct
//! with a polarity boolean would put the whole difference in a field a caller
//! can read the wrong way round, and the difference is the one that decides
//! whether an evidence set means Satisfy or Violate. Two structs make reading it
//! the wrong way round a type error, which is ADR-0031 clause 1's rule applied
//! to the seam rather than to a parameter list. CAT-4 settled that; [`AbsenceIr`]
//! and [`AggregateIr`] keep the same discipline for further families.
//! Keep the accessors and `#[non_exhaustive]`.
//!
//! **Rejected, not deferred:** move `PrecedenceSpecification` /
//! `TaintSpecification` into `compile` and re-export them from `core`.
//!
//! The types stay in `core` because `core` already depends on
//! `sentinel-compile`; the reverse edge is a cycle (ADR-0012, ADR-0025).
//! Compile produces [`MonitorIr`]. Core consumes it via family `TryFrom`s
//! (`TryFrom<&PrecedenceIr>` in `core/src/specification.rs`, `TryFrom<&TaintIr>`
//! in `core/src/taint.rs`, `TryFrom<&AbsenceIr>` in `core/src/absence.rs`,
//! `TryFrom<&AggregateIr>` in `core/src/aggregate.rs`). Each Core decision table
//! lives next to its Core type.
//! That shared reason covers each family. `PrecedenceSpecification` also
//! carries Core-only fields the Evaluator reads (`cold_guards`,
//! `activation_barrier`); `TaintSpecification` now carries the same additive
//! builders so a warming CAT-4 caps authority the way CAT-1 does.
//! `TryFrom<&TaintIr>` stays barrier-free: the IR has no such field.
//!
//! The live edge is already named on the purity allow-list:
//! `core/src/specification.rs`, `core/src/taint.rs` and `core/src/evaluator.rs`
//! all import IR, and `binding_of` is the third production call.

#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.
#![allow(clippy::module_name_repetitions)] // `MonitorIr`/`PrecedenceIr` name the IR on purpose.

use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};

use sentinel_model::{
    AttributeKey, AttributeValue, Event, EventKind, ProducerEvent, SchemaVersion, SourceIdentity,
    SourceTier,
};

/// The monitor a Pattern instance lowers to.
///
/// Read by `core`, never constructed by it: the variants carry public fields but
/// the types are `#[non_exhaustive]`, so only this crate can build one.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum MonitorIr {
    /// A precedence obligation over one Binding attribute.
    Precedence(PrecedenceIr),
    /// A causal-provenance prohibition over one Binding attribute.
    Taint(TaintIr),
    /// An absence obligation over an interval delimited by two Events.
    Absence(AbsenceIr),
    /// A sequence-lookback aggregation obligation over one Binding attribute.
    Aggregate(AggregateIr),
    /// An existence obligation over a Between Scope.
    Existence(ExistenceIr),
}

/// What a lowered monitor declares about a Scope that may never close.
///
/// The IR dual of `crate::catalogue::RetirementPolicy` (ADR-0021's O-03), and a
/// **separate enum on purpose**: this module is the narrow seam a Pattern
/// crosses into the Core, and the Core reads IR rather than catalogue
/// vocabulary. Re-exporting the catalogue enum here would make `core` a
/// consumer of the catalogue, which the module header refuses for every other
/// field.
///
/// Carried on **every** monitor struct and read through
/// [`MonitorIr::retirement`] rather than lowered for one family only. The
/// catalogue declares a policy for every Pattern, so discarding it for CAT-4
/// (or CAT-2 / CAT-5) and re-asserting it as a Core constant would lose a
/// declared fact and duplicate it as a hardcoded family check. With it carried,
/// "a CAT-4 Binding cannot be retired" is a consequence of what its Pattern
/// declares.
///
/// No duration is carried, for the reason `crate::catalogue::RetirementPolicy`
/// gives: the horizon is the periphery's, and it reaches the Core as an input.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum RetirementIr {
    /// The Scope cannot fail to close, so no retirement mechanism applies.
    NotRequired,
    /// The Scope retires when the periphery's declared deadline passes.
    Deadline,
    /// The Scope retires after the periphery's declared quiet window.
    Inactivity,
}

/// Selects Events by kind and by required attribute valuations.
///
/// An Event matches when its kind is equal and every required attribute is
/// present with exactly the required value. Absent attributes never match:
/// nothing here infers a value it did not observe. There is no negation and no
/// range, and #14 does not add either — expressing SagaShop's P3 ("cancel is
/// terminal, except for refunds") needs selector negation, which is a change to
/// the selector language and a separate, reviewed decision.
///
/// # Two consequences of that limit, both load-bearing
///
/// **Every well-formed selector is satisfiable.** `required_attributes` is a
/// map keyed by [`AttributeKey`], so it cannot demand two values for one key
/// and cannot represent a blank key; there is no negation, no disjunction and
/// no range to contradict; and its values are drawn from the same
/// `AttributeValue` an Event carries. So no static check can find a selector
/// that "provably cannot match".
/// `every_selector_this_language_can_express_is_satisfiable_by_some_event` (in
/// `compile/tests/instantiation.rs`) is that argument as an enumeration rather
/// than as this paragraph. **Whoever adds negation or ranges owns the
/// compile-time unsatisfiability check that becomes possible with them**;
/// until then, a trigger that matches nothing is only observable against
/// evidence, which is what [`SettledEvidence`](crate::catalogue::SettledEvidence)
/// and [`TriggerExercise`](crate::catalogue::TriggerExercise) exist for
/// (issue #144).
///
/// **Inclusion between two selectors is exact, for the same reason.**
/// [`SelectorIr::narrowing_divergence`] decides `match(a) ⊆ match(b)` by kind
/// equality and required-attribute submap inclusion, and that is an iff only
/// because a selector is a conjunction of exact equalities over an arbitrary
/// producer attribute map. Whoever adds negation or ranges therefore owns
/// **two** functions, not one: the unsatisfiability check above, and
/// `narrowing_divergence`, which stays sound but stops being exact the moment
/// one selector can exclude a value the other admits (issue #177).
///
/// **The matcher lives here and nowhere else.** `core`'s `EventSelector`
/// delegates to [`SelectorIr::matches`] rather than carrying a second copy.
/// A census in `compile` that disagreed with the Core's own matching would be a
/// worse defect than the silent pass it was written to catch, and ADR-0027 and
/// issue #112 argue the general form: a duplicated classification function in a
/// second crate is structurally unable to stay in agreement. Lookup is one key
/// type on both sides — no `as_str()` / `Borrow<str>` round-trip (issue #203).
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SelectorIr {
    /// The Event kind that must match.
    pub kind: EventKind,
    /// Attribute valuations a matching Event must carry exactly.
    pub required_attributes: BTreeMap<AttributeKey, AttributeValue>,
}

impl SelectorIr {
    /// Selects Events of one kind carrying the given attribute valuations.
    ///
    /// `SelectorIr` is `#[non_exhaustive]`, so a consumer outside this crate
    /// cannot write the struct literal. This is how `core` builds one.
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

    /// Returns whether the producer's kind matches, ignoring attributes.
    ///
    /// This is the one implementation. [`SelectorIr::matches_kind`] is the
    /// Event-shaped delegate, so the monitor path and issue #144's census
    /// cannot grow a second matcher.
    ///
    /// Separate from [`SelectorIr::matches_producer`] because the two answer
    /// different questions and the difference is load-bearing twice over:
    /// `core`'s precedence monitor reads anchor candidates by kind alone, and
    /// issue #144's census separates "the deployment never emits this kind"
    /// from "the kind occurs but the valuation never holds" — different
    /// authoring mistakes with different remediations.
    #[must_use]
    pub fn matches_kind_producer(&self, producer: &ProducerEvent) -> bool {
        producer.kind() == self.kind
    }

    /// Returns whether the Event's kind matches, ignoring attributes.
    #[must_use]
    pub fn matches_kind(&self, event: &Event) -> bool {
        self.matches_kind_producer(&event.producer())
    }

    /// Returns whether the producer matches kind and every required attribute.
    ///
    /// This is the one implementation. [`SelectorIr::matches`] and
    /// [`crate::catalogue::SelectorValue::matches_producer`] both delegate
    /// here, so a census, a monitor, and Candidate adversarial mutation cannot
    /// grow a second matcher (ADR-0027, issue #112).
    #[must_use]
    pub fn matches_producer(&self, producer: &ProducerEvent) -> bool {
        producer_satisfies_selector(&self.kind, &self.required_attributes, producer)
    }

    /// Returns whether the Event matches kind and every required attribute.
    #[must_use]
    pub fn matches(&self, event: &Event) -> bool {
        self.matches_producer(&event.producer())
    }

    /// Returns the first reason this selector matches an Event `wider` does
    /// not, or `None` when every Event this selector matches, `wider` matches
    /// too.
    ///
    /// **Exact for this selector language, not merely conservative.** A
    /// selector is a conjunction of exact equalities, so
    /// `match(self) ⊆ match(wider)` holds exactly when the kinds are equal and
    /// `wider`'s required attributes are a submap of `self`'s:
    ///
    /// - *sufficient* by the matcher's definition
    ///   ([`SelectorIr::matches_producer`]);
    /// - *necessary* because an Event matching `self` can always omit a key only
    ///   `wider` demands, or carry `self`'s value for a key both demand — an
    ///   Event's producer attributes are an arbitrary map, and `compile` refuses
    ///   to legislate a key grammar over them (`catalogue/instance.rs`'s header).
    ///   There is no wildcard kind, so differing kinds share no Event.
    ///
    /// So a returned [`SelectorDivergence`] is a true statement about the two
    /// match sets. It is **not** a statement about any deployment: whether an
    /// Event that tells the two apart ever occurs is a fact this crate cannot
    /// see, and no declared Topology is an input it may read (ADR-0004). That
    /// is why issue #177's analysis treats a divergence as "not proven" rather
    /// than as a refutation.
    ///
    /// Deterministic: the kind is compared first, then `wider`'s required keys
    /// in [`BTreeMap`] order, and the first that diverges is reported. The same
    /// pair always yields the same reason.
    ///
    /// See this type's header for who owns this function when the selector
    /// language grows negation or ranges.
    #[must_use]
    pub fn narrowing_divergence(&self, wider: &Self) -> Option<SelectorDivergence> {
        // No `..`: a field added to `SelectorIr` — a negation, a range — fails
        // to compile here (E0027), which is exactly where this function stops
        // being exact and has to be re-argued (this type's header).
        let Self {
            kind,                // compared: equal
            required_attributes, // compared: `wider`'s are a submap of these
        } = self;
        let Self {
            kind: wider_kind,
            required_attributes: wider_attributes,
        } = wider;
        if kind != wider_kind {
            return Some(SelectorDivergence::KindsDiffer {
                subject: kind.clone(),
                target: wider_kind.clone(),
            });
        }
        for (key, demanded) in wider_attributes {
            match required_attributes.get(key) {
                None => {
                    return Some(SelectorDivergence::RequirementNotCarried {
                        key: key.clone(),
                        demanded: demanded.clone(),
                    });
                }
                Some(carried) if carried != demanded => {
                    return Some(SelectorDivergence::RequirementsConflict {
                        key: key.clone(),
                        subject: carried.clone(),
                        target: demanded.clone(),
                    });
                }
                Some(_) => {}
            }
        }
        None
    }

    /// Returns whether every Event this selector matches is matched by `wider`.
    ///
    /// Delegates to [`SelectorIr::narrowing_divergence`]: one implementation,
    /// for the reason this type's header gives about the matcher.
    #[must_use]
    pub fn is_narrower_or_equal(&self, wider: &Self) -> bool {
        self.narrowing_divergence(wider).is_none()
    }
}

/// Why one selector is not narrower than or equal to another.
///
/// Returned by [`SelectorIr::narrowing_divergence`]. The field names are
/// neutral because this type is a statement about the selector language and
/// knows nothing of Candidates or deployed Specifications: `subject` is the
/// selector claimed narrower (the receiver), `target` the one claimed wider
/// (the argument). Which side of a subsumption question each one is gets
/// resolved where a reviewer reads it — `crate::Undecided`'s `Display` — not
/// here.
///
/// Every variant is a true statement about the two match sets and never one
/// about a deployment; see [`SelectorIr::narrowing_divergence`].
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SelectorDivergence {
    /// The two selectors name different Event kinds. There is no wildcard kind
    /// in this language, so no Event matches both.
    KindsDiffer {
        /// The kind the selector claimed narrower names.
        subject: EventKind,
        /// The kind the selector claimed wider names.
        target: EventKind,
    },
    /// `target` demands an attribute `subject` does not, so an Event that
    /// omits it matches `subject` and not `target`.
    RequirementNotCarried {
        /// The key only `target` demands.
        key: AttributeKey,
        /// The value `target` demands under it.
        demanded: AttributeValue,
    },
    /// Both demand the key at different values, so no Event matches both.
    RequirementsConflict {
        /// The contested key.
        key: AttributeKey,
        /// The value `subject` demands.
        subject: AttributeValue,
        /// The value `target` demands.
        target: AttributeValue,
    },
}

impl Display for SelectorDivergence {
    /// Renders the divergence in neutral subject/target terms.
    ///
    /// `AttributeValue` has no `Display` (`sentinel_model`), so values are
    /// rendered with `{:?}`; the rendering is a reviewer-facing string and is
    /// not parsed anywhere.
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::KindsDiffer { subject, target } => write!(
                formatter,
                "the selector claimed narrower names Event kind {subject} and the one claimed \
                 wider names {target}"
            ),
            Self::RequirementNotCarried { key, demanded } => write!(
                formatter,
                "the selector claimed wider demands {key} = {demanded:?}, which the one claimed \
                 narrower does not"
            ),
            Self::RequirementsConflict {
                key,
                subject,
                target,
            } => write!(
                formatter,
                "the selector claimed narrower demands {key} = {subject:?} and the one claimed \
                 wider demands {key} = {target:?}"
            ),
        }
    }
}

/// Kind equality plus exact required-attribute valuation. Shared by
/// [`SelectorIr`] and [`crate::catalogue::SelectorValue`].
pub(crate) fn producer_satisfies_selector(
    kind: &EventKind,
    required_attributes: &BTreeMap<AttributeKey, AttributeValue>,
    producer: &ProducerEvent,
) -> bool {
    if producer.kind() != *kind {
        return false;
    }
    let attributes = producer.attributes();
    required_attributes
        .iter()
        .all(|(key, value)| attributes.get(key) == Some(value))
}

/// Reads the Binding producer evidence names under a Binding attribute, if it names one.
///
/// **The one implementation of this in the workspace.** [`binding_of`] is the
/// Event-shaped delegate; `core`'s `PrecedenceSpecification::binding_of` and
/// `binding_of_producer` both reach here, for the reason [`SelectorIr`]'s docs
/// give about selector matching: a second copy in a second crate is
/// structurally unable to stay in agreement (ADR-0027, issue #112), and
/// `PatternInstance::exercise` needs to ask this question inside `compile`.
///
/// It is stricter than "does the producer carry this key?", which is why
/// reproducing it by eye would have been wrong. An attribute holding a
/// [`AttributeValue::Bool`], an [`AttributeValue::Integer`] or an
/// [`AttributeValue::Double`] under the Binding attribute names **no** Binding:
/// a Binding is a [`AttributeValue::String`] or it is nothing. A census that
/// counted key presence would report obligations the Verification Core never
/// raises.
///
/// Lookup is keyed by [`AttributeKey`] against the producer's attribute map —
/// one type on both sides (issue #203). Callers that still hold a public IR
/// `String` (Pattern `binding_attribute`) convert with [`AttributeKey::new`];
/// a blank/poisoned IR field yields `None` (fail-closed for census).
#[must_use]
pub fn binding_of_producer(
    binding_attribute: &AttributeKey,
    producer: &ProducerEvent,
) -> Option<String> {
    match producer.attributes().get(binding_attribute) {
        Some(AttributeValue::String(value)) => Some(value.clone()),
        _ => None,
    }
}

/// Reads the Binding an Event names under a Binding attribute, if it names one.
#[must_use]
pub fn binding_of(binding_attribute: &AttributeKey, event: &Event) -> Option<String> {
    binding_of_producer(binding_attribute, &event.producer())
}

/// Converts a public IR Binding-attribute `String` for map lookup.
///
/// Pattern IR identity fields stay `String` (#21 / #189). A blank or poisoned
/// value is unrepresentable as [`AttributeKey`], so census treats it as no
/// Binding rather than panicking.
fn attribute_key_from_ir(binding_attribute: &str) -> Option<AttributeKey> {
    AttributeKey::new(binding_attribute.to_owned()).ok()
}

impl PrecedenceIr {
    /// Reads the Binding this monitor would file an Event under.
    ///
    /// Thin wrapper over [`binding_of`] supplying this monitor's own Binding
    /// attribute, so a caller cannot pair one monitor's attribute with another's
    /// Events by accident.
    #[must_use]
    pub fn binding_of(&self, event: &Event) -> Option<String> {
        binding_of(&attribute_key_from_ir(&self.binding_attribute)?, event)
    }
}

impl TaintIr {
    /// Reads the Binding this monitor would file an Event under.
    #[must_use]
    pub fn binding_of(&self, event: &Event) -> Option<String> {
        binding_of(&attribute_key_from_ir(&self.binding_attribute)?, event)
    }
}

impl AbsenceIr {
    /// Reads the Binding this monitor would file an Event under.
    #[must_use]
    pub fn binding_of(&self, event: &Event) -> Option<String> {
        binding_of(&attribute_key_from_ir(&self.binding_attribute)?, event)
    }
}

impl AggregateIr {
    /// Reads the Binding this monitor would file an Event under.
    #[must_use]
    pub fn binding_of(&self, event: &Event) -> Option<String> {
        binding_of(&attribute_key_from_ir(&self.binding_attribute)?, event)
    }
}

impl ExistenceIr {
    /// Reads the Binding this monitor would file an Event under.
    #[must_use]
    pub fn binding_of(&self, event: &Event) -> Option<String> {
        binding_of(&attribute_key_from_ir(&self.binding_attribute)?, event)
    }
}

impl MonitorIr {
    /// Returns the precedence monitor, if that is what this Pattern lowered to.
    ///
    /// `MonitorIr` is `#[non_exhaustive]`, so a consumer outside this crate
    /// cannot write an irrefutable `let` over its variants. This is the accessor
    /// that kept adding further families additive for them — and that makes a
    /// taint, absence or aggregate monitor read as `None` here instead of being
    /// mistaken for a precedence obligation whose polarity is inverted.
    /// `core/tests/taint_decision_table.rs` pins the CAT-4 half.
    #[must_use]
    pub const fn as_precedence(&self) -> Option<&PrecedenceIr> {
        match self {
            Self::Precedence(precedence) => Some(precedence),
            Self::Taint(_) | Self::Absence(_) | Self::Aggregate(_) | Self::Existence(_) => None,
        }
    }

    /// Returns the taint monitor, if that is what this Pattern lowered to.
    #[must_use]
    pub const fn as_taint(&self) -> Option<&TaintIr> {
        match self {
            Self::Taint(taint) => Some(taint),
            Self::Precedence(_) | Self::Absence(_) | Self::Aggregate(_) | Self::Existence(_) => {
                None
            }
        }
    }

    /// Returns the absence monitor, if that is what this Pattern lowered to.
    #[must_use]
    pub const fn as_absence(&self) -> Option<&AbsenceIr> {
        match self {
            Self::Absence(absence) => Some(absence),
            Self::Precedence(_) | Self::Taint(_) | Self::Aggregate(_) | Self::Existence(_) => None,
        }
    }

    /// Returns the aggregate monitor, if that is what this Pattern lowered to.
    #[must_use]
    pub const fn as_aggregate(&self) -> Option<&AggregateIr> {
        match self {
            Self::Aggregate(aggregate) => Some(aggregate),
            Self::Precedence(_) | Self::Taint(_) | Self::Absence(_) | Self::Existence(_) => None,
        }
    }

    /// Returns the existence monitor, if that is what this Pattern lowered to.
    #[must_use]
    pub const fn as_existence(&self) -> Option<&ExistenceIr> {
        match self {
            Self::Existence(existence) => Some(existence),
            Self::Precedence(_) | Self::Taint(_) | Self::Absence(_) | Self::Aggregate(_) => None,
        }
    }

    /// Returns the Binding attribute whichever monitor this is reads.
    ///
    /// Every monitor family has exactly one, because a Slice is what a Verdict
    /// lives in and ADR-0016's differentiator is that the prohibited sequence is
    /// *data-parametric*. Exposed as one accessor so a caller that needs only
    /// the Binding — `FilterSource::binding_attribute_keys` is the one in this
    /// crate — does not have to grow an arm per family and cannot forget one.
    #[must_use]
    pub const fn binding_attribute(&self) -> &String {
        match self {
            Self::Precedence(precedence) => &precedence.binding_attribute,
            Self::Taint(taint) => &taint.binding_attribute,
            Self::Absence(absence) => &absence.binding_attribute,
            Self::Aggregate(aggregate) => &aggregate.binding_attribute,
            Self::Existence(existence) => &existence.binding_attribute,
        }
    }

    /// Returns the sources whichever monitor this is cannot be trusted without.
    #[must_use]
    pub const fn required_sources(&self) -> &BTreeMap<SourceIdentity, SourceTier> {
        match self {
            Self::Precedence(precedence) => &precedence.required_sources,
            Self::Taint(taint) => &taint.required_sources,
            Self::Absence(absence) => &absence.required_sources,
            Self::Aggregate(aggregate) => &aggregate.required_sources,
            Self::Existence(existence) => &existence.required_sources,
        }
    }

    /// Returns what this monitor declares about a Scope that may never close.
    ///
    /// One accessor beside [`MonitorIr::required_sources`], for the same
    /// reason: the Core reads a declaration uniformly rather than growing an
    /// arm per family and forgetting one. It is what makes CAT-4's refusal to
    /// retire come from the Pattern's own declaration (ADR-0021's O-03).
    #[must_use]
    pub const fn retirement(&self) -> RetirementIr {
        match self {
            Self::Precedence(precedence) => precedence.retirement,
            Self::Taint(taint) => taint.retirement,
            Self::Absence(absence) => absence.retirement,
            Self::Aggregate(aggregate) => aggregate.retirement,
            Self::Existence(existence) => existence.retirement,
        }
    }
}

/// A precedence obligation: every trigger must have a requirement in its causal past.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct PrecedenceIr {
    /// Identifier of the Specification this instance is promoted under.
    pub specification_id: String,
    /// Version of the Specification text.
    pub specification_version: String,
    /// Producer schema contract the selectors are written against.
    ///
    /// A producer-contract fact, not a Pattern parameter — see
    /// [`ProducerContract`](crate::catalogue::ProducerContract).
    pub schema_version: SchemaVersion,
    /// Attribute whose value names a Binding.
    pub binding_attribute: String,
    /// Events that create the obligation.
    pub trigger: SelectorIr,
    /// Events that discharge it.
    pub requirement: SelectorIr,
    /// Sources the conclusion is not trustworthy without, and the **minimum**
    /// tier each must hold.
    ///
    /// Keyed by source rather than by evidence role: two roles may bind the same
    /// source, and what the conclusion needs from that source is one tier, not
    /// two. Where roles disagree the stricter minimum is kept, because a source
    /// required at anchor tier by any role is required at anchor tier.
    pub required_sources: BTreeMap<SourceIdentity, SourceTier>,
    /// What the Pattern declares about a Scope that may never close.
    ///
    /// Lowered from the Pattern's own `retirement` declaration; read through
    /// [`MonitorIr::retirement`]. See [`RetirementIr`].
    pub retirement: RetirementIr,
}

/// A causal-provenance prohibition: no action may have a falsifying ancestor.
///
/// CAT-4 `provenance.taint`. The obligation is: for every action Event `t` in a
/// Binding's Slice, **no** Event matching [`TaintIr::untrusted_content`] lies in
/// `t`'s causal past. That is ADR-0016's "or was influenced by untrusted
/// content", and it is [`PrecedenceIr`]'s operator with the polarity inverted —
/// `AG(action -> !EP(untrusted))` against `AG(trigger -> EP(requirement))`.
///
/// # Why the polarity is a different struct and not a field
///
/// The inversion is not cosmetic. It moves which row of the monitor's decision
/// table is the *default*: a precedence obligation passes on a positive witness
/// and a taint prohibition passes on an **absence**, so a condition folded into
/// the violation row falls through to `Satisfy` here and to `Inconclusive`
/// there. `core/src/taint.rs` carries that argument and the table it forces.
/// A shared struct with a `negated: bool` would make the two reachable through
/// one `match` arm, which is exactly the arm that must not exist.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct TaintIr {
    /// Identifier of the Specification this instance is promoted under.
    pub specification_id: String,
    /// Version of the Specification text.
    pub specification_version: String,
    /// Producer schema contract the selectors are written against.
    pub schema_version: SchemaVersion,
    /// Attribute whose value names a Binding.
    pub binding_attribute: String,
    /// Events whose provenance is constrained, and which raise the obligation.
    ///
    /// The obligation-raising selector, which is what
    /// [`PatternInstance::exercise`](crate::catalogue::PatternInstance::exercise)
    /// censuses. Censusing [`TaintIr::untrusted_content`] instead would report a
    /// *clean* deployment as never exercised, which is issue #144's trap in the
    /// shape `compile/src/catalogue/exercise.rs` warns about.
    pub action: SelectorIr,
    /// Events marking the ingestion of untrusted content.
    ///
    /// Their presence in an action's causal past falsifies the property. This is
    /// the selector that must never match *there* — it may match freely
    /// elsewhere in the Trace, and usually does.
    pub untrusted_content: SelectorIr,
    /// Sources the conclusion is not trustworthy without, and the **minimum**
    /// tier each must hold.
    ///
    /// Keyed by source for [`PrecedenceIr`]'s reason: two roles may bind one
    /// source, and what the conclusion needs from it is one tier, not two.
    pub required_sources: BTreeMap<SourceIdentity, SourceTier>,
    /// What the Pattern declares about a Scope that may never close.
    ///
    /// [`RetirementIr::NotRequired`] for CAT-4, because the action Event whose
    /// provenance is constrained *is* the Scope's close. Carried rather than
    /// assumed: it is what makes retiring a CAT-4 Binding inadmissible because
    /// the catalogue says so. See [`RetirementIr`].
    pub retirement: RetirementIr,
}

/// An absence obligation: no veto may occur inside an open interval.
///
/// CAT-2 `absence.after-until`. The obligation is: for every Event matching
/// [`AbsenceIr::until`] in a Binding's Slice, **no** Event matching
/// [`AbsenceIr::veto`] lies in the open-closed interval `(after, until]` of that
/// Binding — after the opening delimiter [`AbsenceIr::after`], up to and
/// including the obligation-raising close. The Scope closes at `until`, which
/// is what licenses [`RetirementPolicy::NotRequired`](crate::catalogue::RetirementPolicy::NotRequired)
/// without inventing a `ScopeKind::Between` carrying selectors on the enum.
///
/// # Why the census keys on `until` and never on `veto`
///
/// `veto` is the selector that must never match *inside the interval*. Censusing
/// it would report a clean deployment — one that never emitted a veto — as never
/// exercised, which is issue #144's trap. The obligation is raised when `until`
/// occurs; that is what
/// [`PatternInstance::exercise`](crate::catalogue::PatternInstance::exercise)
/// censuses. `after` is a delimiter present in evidence and is reported as the
/// interval's opening count, not as the thing that was measured.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AbsenceIr {
    /// Identifier of the Specification this instance is promoted under.
    pub specification_id: String,
    /// Version of the Specification text.
    pub specification_version: String,
    /// Producer schema contract the selectors are written against.
    pub schema_version: SchemaVersion,
    /// Attribute whose value names a Binding.
    pub binding_attribute: String,
    /// Events that open the interval the absence is quantified over.
    pub after: SelectorIr,
    /// Events that close the interval and raise the absence obligation.
    ///
    /// The obligation-raising selector, which is what
    /// [`PatternInstance::exercise`](crate::catalogue::PatternInstance::exercise)
    /// censuses. Censusing [`AbsenceIr::veto`] instead would report a *clean*
    /// deployment as never exercised.
    pub until: SelectorIr,
    /// Events prohibited inside `(after, until]`.
    ///
    /// Their presence inside the open interval falsifies the property. They may
    /// match freely outside it, and usually do.
    pub veto: SelectorIr,
    /// Sources the conclusion is not trustworthy without, and the **minimum**
    /// tier each must hold.
    ///
    /// Keyed by source for [`PrecedenceIr`]'s reason: two roles may bind one
    /// source, and what the conclusion needs from it is one tier, not two.
    pub required_sources: BTreeMap<SourceIdentity, SourceTier>,
    /// What the Pattern declares about a Scope that may never close.
    ///
    /// [`RetirementIr::NotRequired`] for CAT-2, because the Scope closes at
    /// `until`. Carried rather than assumed so `MonitorIr::retirement` stays
    /// family-agnostic. See [`RetirementIr`].
    pub retirement: RetirementIr,
}

/// A sequence-lookback aggregation obligation: member count must stay < threshold.
///
/// CAT-5 `aggregation.count-within`. The obligation is: for every Event matching
/// [`AggregateIr::trigger`] in a Binding's Slice, the count of Events matching
/// [`AggregateIr::member`] inside a **sequence** lookback of
/// [`AggregateIr::lookback_sequences`] positions must stay strictly below
/// [`AggregateIr::threshold`]. Lookback is sequence positions along causal
/// evidence order — not wall-clock duration, not Scope, not Deadline.
///
/// # Why the census keys on `trigger` and never on `member`
///
/// `member` is the selector that is *counted*. Censusing it would report a clean
/// under-threshold deployment — one that never emitted a counted member — as
/// never exercised, which is issue #144's trap. The obligation is raised when
/// `trigger` (the assessment Event) occurs; that is what
/// [`PatternInstance::exercise`](crate::catalogue::PatternInstance::exercise)
/// censuses. Member matches are reported as the counterpart count
/// ([`CounterpartMatches::Falsifying`](crate::catalogue::CounterpartMatches::Falsifying)
/// — the population a cold `Fails` would be drawn from once Coverage closes).
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct AggregateIr {
    /// Identifier of the Specification this instance is promoted under.
    pub specification_id: String,
    /// Version of the Specification text.
    pub specification_version: String,
    /// Producer schema contract the selectors are written against.
    pub schema_version: SchemaVersion,
    /// Attribute whose value names a Binding.
    pub binding_attribute: String,
    /// Events that raise the aggregation obligation (assessment / privileged action).
    ///
    /// The obligation-raising selector, which is what
    /// [`PatternInstance::exercise`](crate::catalogue::PatternInstance::exercise)
    /// censuses. Censusing [`AggregateIr::member`] instead would report a *clean*
    /// under-threshold deployment as never exercised.
    pub trigger: SelectorIr,
    /// Events counted inside the sequence lookback.
    ///
    /// Their count is compared to [`AggregateIr::threshold`] once the cold path
    /// holds a complete lookback. They may match freely and usually do.
    pub member: SelectorIr,
    /// Sequence positions of causal evidence included when counting members.
    ///
    /// Not wall-clock duration. Maps to cold-path `LookbackBound`.
    pub lookback_sequences: u64,
    /// Exclusive upper bound on the member count inside the lookback
    /// (Holds when count < threshold under a complete lookback).
    pub threshold: u64,
    /// Sources the conclusion is not trustworthy without, and the **minimum**
    /// tier each must hold.
    ///
    /// Keyed by source for [`PrecedenceIr`]'s reason: two roles may bind one
    /// source, and what the conclusion needs from it is one tier, not two.
    pub required_sources: BTreeMap<SourceIdentity, SourceTier>,
    /// What the Pattern declares about a Scope that may never close.
    ///
    /// [`RetirementIr::NotRequired`] for CAT-5: lookback is a sequence cold
    /// parameter, not a Scope that can fail to close. Carried for
    /// retirement-free workload byte-identity and family-agnostic
    /// [`MonitorIr::retirement`]. See [`RetirementIr`].
    pub retirement: RetirementIr,
}

/// An existence obligation: a required Event occurs inside a Between Scope.
///
/// CAT-3 `existence.before-scope-close`. The obligation is: for every Event
/// matching [`ExistenceIr::open`] in a Binding's Slice, **at least one** Event
/// matching [`ExistenceIr::required`] lies in the open-closed interval
/// `(open, close]` before the Scope closes at [`ExistenceIr::close`].
///
/// # Why the census keys on `open` and never on `required`
///
/// `required` is the selector that must match *inside the interval*. Censusing
/// it would report a Trace full of compensations with no saga open as exercised,
/// which is issue #144's trap in the opposite polarity. The obligation is raised
/// when `open` occurs.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ExistenceIr {
    /// Identifier of the Specification this instance is promoted under.
    pub specification_id: String,
    /// Version of the Specification text.
    pub specification_version: String,
    /// Producer schema contract the selectors are written against.
    pub schema_version: SchemaVersion,
    /// Attribute whose value names a Binding.
    pub binding_attribute: String,
    /// Events that open the Between Scope and raise the existence obligation.
    pub open: SelectorIr,
    /// Events that must occur at least once inside `(open, close]`.
    pub required: SelectorIr,
    /// Events that close the Between Scope.
    pub close: SelectorIr,
    /// Sources the conclusion is not trustworthy without, and the **minimum**
    /// tier each must hold.
    pub required_sources: BTreeMap<SourceIdentity, SourceTier>,
    /// What the Pattern declares about a Scope that may never close.
    ///
    /// [`RetirementIr::Deadline`] for catalogued CAT-3. See [`RetirementIr`].
    pub retirement: RetirementIr,
}

#[cfg(test)]
mod selector_inclusion {
    //! The inclusion algebra beside the type, tied back to the matcher.
    //!
    //! `selector_inclusion_agrees_with_the_matcher_on_a_witness_event` is the
    //! test that matters: it builds, for every ordered pair in a small matrix,
    //! the Event that separates the two selectors whenever
    //! [`SelectorIr::narrowing_divergence`] says one exists, and a matching
    //! Event the wider selector must accept whenever it says none does. The
    //! algebra and [`SelectorIr::matches_producer`] therefore cannot drift
    //! (ADR-0027 applied to the new predicate).

    use std::collections::{BTreeMap, BTreeSet};

    use sentinel_model::{
        AttributeDouble, AttributeKey, AttributeValue, ClockDomainId, EventId, EventKind,
        FilterEpoch, OccurrenceTime, ProducerEvent, SchemaVersion, Sensitivity, SequenceEpoch,
        SequenceNumber, SourceCapabilities,
    };

    use super::{SelectorDivergence, SelectorIr};

    fn key(name: &str) -> AttributeKey {
        AttributeKey::new(name).expect("constant")
    }

    fn kind(name: &str) -> EventKind {
        EventKind::new(name).expect("constant")
    }

    fn text(value: &str) -> AttributeValue {
        AttributeValue::String(value.to_owned())
    }

    fn selector(kind_name: &str, attributes: &[(&str, AttributeValue)]) -> SelectorIr {
        SelectorIr::new(
            kind(kind_name),
            attributes
                .iter()
                .map(|(name, value)| (key(name), value.clone()))
                .collect(),
        )
    }

    /// The Event carrying exactly what `selector` demands and nothing else.
    ///
    /// Whenever `narrowing_divergence` reports a divergence, this Event is the
    /// separating witness: it matches the claimed-narrower selector by
    /// construction, and omits or contradicts whatever the wider one demands.
    fn minimal_witness(selector: &SelectorIr) -> ProducerEvent {
        ProducerEvent::new(
            EventId::new("witness").expect("constant"),
            SequenceEpoch::new(1),
            SequenceNumber::new(1),
            SchemaVersion::new("example-contract-v1").expect("constant"),
            FilterEpoch::new(1),
            selector.kind.clone(),
            OccurrenceTime::new(
                ClockDomainId::new("example-producer").expect("constant"),
                1,
                0,
            ),
            selector.required_attributes.clone(),
            SourceCapabilities::new([]),
            Sensitivity::Internal,
            BTreeSet::new(),
        )
        .expect("a well-formed producer Event")
    }

    /// Every value shape an attribute can hold, as in
    /// `every_selector_this_language_can_express_is_satisfiable_by_some_event`.
    fn values() -> Vec<AttributeValue> {
        vec![
            text("s"),
            AttributeValue::Bool(true),
            AttributeValue::Integer(-7),
            AttributeValue::Double(AttributeDouble::try_from(1.5).expect("finite")),
            AttributeValue::Bytes(vec![0, 1, 2]),
            AttributeValue::Array(vec![AttributeValue::Integer(1)]),
            AttributeValue::KeyValueList(BTreeMap::from([(
                key("inner"),
                AttributeValue::Bool(false),
            )])),
        ]
    }

    /// A small matrix: two kinds, and required maps over two keys drawn from
    /// every value shape, including the empty map and conflicting values.
    fn matrix() -> Vec<SelectorIr> {
        let values = values();
        let mut selectors = Vec::new();
        for kind_name in ["example.a", "example.b"] {
            selectors.push(selector(kind_name, &[]));
            for value in &values {
                selectors.push(selector(kind_name, &[("example.x", value.clone())]));
                selectors.push(selector(kind_name, &[("example.y", value.clone())]));
                selectors.push(selector(
                    kind_name,
                    &[("example.x", value.clone()), ("example.y", text("s"))],
                ));
            }
        }
        selectors
    }

    #[test]
    fn a_selector_is_narrower_than_or_equal_to_itself() {
        for selector in matrix() {
            assert_eq!(selector.narrowing_divergence(&selector), None);
            assert!(selector.is_narrower_or_equal(&selector));
        }
    }

    #[test]
    fn dropping_a_required_attribute_widens_a_selector() {
        let narrow = selector(
            "example.a",
            &[("example.x", text("eu")), ("example.y", text("acme"))],
        );
        let wide = selector("example.a", &[("example.x", text("eu"))]);
        let widest = selector("example.a", &[]);

        assert!(narrow.is_narrower_or_equal(&wide));
        assert!(wide.is_narrower_or_equal(&widest));
        assert!(narrow.is_narrower_or_equal(&widest));
        assert_eq!(
            wide.narrowing_divergence(&narrow),
            Some(SelectorDivergence::RequirementNotCarried {
                key: key("example.y"),
                demanded: text("acme"),
            }),
            "adding a requirement narrows, so the wider selector is not included in it"
        );
        assert!(!widest.is_narrower_or_equal(&narrow));
    }

    #[test]
    fn narrowing_divergence_and_is_narrower_or_equal_never_disagree() {
        let matrix = matrix();
        for subject in &matrix {
            for target in &matrix {
                assert_eq!(
                    subject.is_narrower_or_equal(target),
                    subject.narrowing_divergence(target).is_none(),
                    "{subject:?} against {target:?}"
                );
            }
        }
    }

    #[test]
    fn kind_divergence_is_reported_before_attribute_divergence() {
        let subject = selector("example.a", &[("example.x", text("eu"))]);
        let target = selector(
            "example.b",
            &[("example.x", text("us")), ("example.y", text("acme"))],
        );
        assert_eq!(
            subject.narrowing_divergence(&target),
            Some(SelectorDivergence::KindsDiffer {
                subject: kind("example.a"),
                target: kind("example.b"),
            })
        );
    }

    #[test]
    fn the_first_diverging_attribute_key_is_reported_in_btreemap_order() {
        // `target` demands three keys. `subject` carries `example.a` at the
        // same value, does not carry `example.b`, and conflicts on `example.c`.
        // BTreeMap order puts `example.b` first among the two divergences, so a
        // not-carried requirement is reported ahead of a later conflict.
        let subject = selector(
            "example.kind",
            &[("example.a", text("1")), ("example.c", text("mine"))],
        );
        let target = selector(
            "example.kind",
            &[
                ("example.c", text("theirs")),
                ("example.a", text("1")),
                ("example.b", text("2")),
            ],
        );
        assert_eq!(
            subject.narrowing_divergence(&target),
            Some(SelectorDivergence::RequirementNotCarried {
                key: key("example.b"),
                demanded: text("2"),
            })
        );

        // Carry `example.b` and the conflict on `example.c` is what is left.
        let carrying = selector(
            "example.kind",
            &[
                ("example.a", text("1")),
                ("example.b", text("2")),
                ("example.c", text("mine")),
            ],
        );
        assert_eq!(
            carrying.narrowing_divergence(&target),
            Some(SelectorDivergence::RequirementsConflict {
                key: key("example.c"),
                subject: text("mine"),
                target: text("theirs"),
            })
        );
    }

    /// **Ties the inclusion algebra to the matcher, in both directions.**
    ///
    /// Included: the claimed-wider selector must accept an Event the narrower
    /// one matches. Not included: the minimal witness of the narrower selector
    /// must match it and be refused by the wider one — which is the
    /// *necessity* half of the exactness claim, executed rather than argued.
    #[test]
    fn selector_inclusion_agrees_with_the_matcher_on_a_witness_event() {
        let matrix = matrix();
        let mut included = 0_usize;
        let mut separated = 0_usize;
        for subject in &matrix {
            let witness = minimal_witness(subject);
            assert!(
                subject.matches_producer(&witness),
                "the minimal witness of {subject:?} must match it"
            );
            for target in &matrix {
                match subject.narrowing_divergence(target) {
                    None => {
                        included += 1;
                        assert!(
                            target.matches_producer(&witness),
                            "{subject:?} was judged narrower than {target:?}, yet an Event it \
                             matches is refused by the wider selector"
                        );
                    }
                    Some(divergence) => {
                        separated += 1;
                        assert!(
                            !target.matches_producer(&witness),
                            "{subject:?} was judged not narrower than {target:?} ({divergence}), \
                             yet its minimal witness matches the wider selector"
                        );
                    }
                }
            }
        }
        // Anti-vacuity: both halves of the claim were actually exercised.
        assert!(
            included > matrix.len(),
            "only reflexive pairs were included"
        );
        assert!(separated > 0, "no pair was separated");
    }
}
