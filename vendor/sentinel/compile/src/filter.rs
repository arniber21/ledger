//! Attribute-level Event Filter compiler (ADR-0006, O-08).
//!
//! Compiles a set of [`FilterSource`]s into a validated
//! [`EventFilter`](sentinel_model::EventFilter) — an attribute-level
//! over-approximation of the Events each active/warming Specification could
//! depend on. Probes push this Filter down so they ship only relevant Events,
//! without sampling. The compiler emits **Shadow/Warming/ShipAsync** non-enforcing
//! defaults with empty activation watermarks; publication stamps the barrier.
//!
//! ## Soundness contract
//!
//! A Specification's **relevant causes** are the Events it could match or slice
//! on. The compiled `SpecificationFilter` is SOUND iff:
//!
//! - **S1**: `event_kinds ⊇ { d.kind : d ∈ dependencies }` — every Event kind a
//!   selector could match is admitted, so a matching Event is never dropped.
//! - **S2**: `projected_attribute_keys ⊇ (⋃ d.attribute_keys) ∪ binding_attribute_keys`
//!   — every attribute the Core reads to match or bind is retained, so slicing
//!   and binding-of survive the projection.
//! - **S3**: `causal_predecessor_ids` is never trimmed — it is a top-level
//!   `ProducerEvent` field, not an attribute; the consumer preserves it
//!   verbatim. The compiler does not touch causality.
//!
//! Then no relevant cause is dropped: an Event matching any selector has its
//! kind in `event_kinds` (admitted); every attribute the Core needs is in
//! `projected_attribute_keys` (retained); causality preserved wholesale. The
//! Filter MAY admit irrelevant Events (extra kinds/keys) — the permitted
//! over-approximation. [`project`] builds `event_kinds`/`projected_attribute_keys`
//! as exactly these unions, so S1/S2 hold by construction; the tests assert them
//! post-hoc against a future misguided width cap.
//!
//! ## Trust boundary
//!
//! The compiler proves soundness GIVEN declared dependencies. A Specification
//! that omits a dependency it actually reads is unsound and **undetectable**
//! here — that is a Pattern-authoring bug, tested per-Pattern in #14.
//! ADR-0006's obligation on the compiler is: given the spec's declared
//! selectors, never drop what they reference. This is the over-approximation-
//! via-projection seam #14 Patterns plug into.
//!
//! ## Defaults and why they validate
//!
//! Every emitted `SpecificationFilter` uses: `evaluation_mode = Shadow`,
//! `readiness = Warming`, `delivery_mode = ShipAsync`, `fail_mode = FailOpen`
//! (REQUIRED unconditionally — `parse_fail_mode` rejects `Unspecified` for
//! every spec inside `EventFilter::try_from`, it is NOT Enforce-gated),
//! `latency_budget_nanoseconds = None`, `enforcement_gates = None`,
//! `activation_watermarks = []`. The empty activation-barrier set is VALID for
//! *unpublished Warming*: `validate_activation_barrier` now runs on the
//! Readiness axis, and Warming with empty watermarks remains the compile-filter
//! soundness path. A published Filter is stamped later via
//! [`activation_watermarks_for`]. `baseline_evidence` is always ON
//! (`{source_health: true, completeness_signals: true}`) because validation
//! rejects a disabled baseline. `target_filter_epoch` is stamped to the
//! compile epoch (validation requires equality with the envelope epoch).
//!
//! ## Empty-keys consumer contract (load-bearing)
//!
//! A Specification whose dependencies declare no attribute keys AND whose
//! `binding_attribute_keys` is empty compiles to an **empty**
//! `projected_attribute_keys`. The TS reference consumer (`apply-filter.ts`)
//! treats an empty `projected_attribute_keys` as **retain-all**: every
//! attribute on an admitted Event is kept. This is load-bearing — the compiler
//! must emit empty (not fill with a sentinel) so the consumer's retain-all
//! branch fires. `empty_projected_keys_emits_empty_test` pins this.

#![allow(clippy::module_name_repetitions, clippy::doc_markdown)]

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use sentinel_model::wire::model::v1 as raw_model;
use sentinel_model::{
    AttributeKey, EventFilter, EventFilterError, EventKind, FilterEpoch, ObservationTime,
    SequenceEpoch, SequenceNumber, SourceIdentity,
};

use crate::width::{FilterWidthBudget, WidthReport};

/// One Event kind the Specification could depend on, plus the attribute keys it
/// slices on for that kind.
///
/// A dependency's `attribute_keys` are the keys the Core reads when matching or
/// slicing Events of `kind`; the compiler projects their union into
/// `projected_attribute_keys` so retained Events still carry them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventDependency {
    /// Event kind this dependency matches.
    pub kind: EventKind,
    /// Attribute keys the Core reads for Events of this kind.
    pub attribute_keys: BTreeSet<AttributeKey>,
}

/// Stable identity of one compiled SpecificationFilter.
///
/// `(specification_id, specification_version)`; uniqueness is enforced across a
/// single [`compile_filter`] call.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct FilterIdentity {
    /// Specification identifier.
    pub specification_id: String,
    /// Specification version.
    pub specification_version: String,
}

/// The seam a Specification implements to expose what Events it could depend on.
///
/// The compiler owns the union, the soundness proof, the width report, and the
/// proto assembly; each spec owns its projection (the seam #14 Patterns plug
/// into). This is NOT the proto `SpecificationFilter` — that mixes relevance
/// with enforcement state (#19).
///
/// # Trust boundary
///
/// The compiler proves soundness GIVEN the dependencies declared here. A spec
/// that hides a dependency it reads is unsound and undetectable (Pattern-
/// authoring bug, tested per-Pattern in #14). The compiler's obligation
/// (ADR-0006) is: given these declared selectors, never drop what they
/// reference — see the module docs for the S1/S2/S3 invariant.
///
/// # Why `Sync`
///
/// Control-plane futures (`sentinel_api::publish`, `assemble_event_filter`,
/// Candidate review and Warm) hold `&[&dyn FilterSource]` across `.await`.
/// `&T` is `Send` only when `T: Sync`, so without this supertrait each of those
/// futures is `!Send` and cannot be awaited from a multi-threaded executor
/// such as the periphery's handlers (#277). Implementors are plain data, so
/// the bound costs them nothing. `sentinel-api`'s `send_bounds` module checks
/// at compile time that the futures stay `Send`.
pub trait FilterSource: Sync {
    /// Returns the stable identity of this Specification.
    fn filter_identity(&self) -> FilterIdentity;
    /// Returns the Events this Specification could match or slice on.
    fn dependencies(&self) -> Vec<EventDependency>;
    /// Returns the binding/identity attribute keys projected from every admitted
    /// Event (the free-variable attribute and any slicing keys not already in a
    /// dependency's `attribute_keys`).
    fn binding_attribute_keys(&self) -> Vec<AttributeKey>;
    /// Returns the sources this Specification's conclusion is not trustworthy
    /// without.
    fn required_sources(&self) -> Vec<SourceIdentity>;
    /// Returns the per-Pattern declared width budget (ADR-0008).
    fn width_budget(&self) -> FilterWidthBudget;
}

/// Failure raised by [`compile_filter`].
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CompileError {
    /// No FilterSources were supplied.
    EmptySources,
    /// Two FilterSources share the same [`FilterIdentity`].
    DuplicateIdentity(FilterIdentity),
    /// A FilterSource projected zero Event kinds (a spec that matches nothing).
    EmptyEventKinds(FilterIdentity),
    /// A required source has no SequenceEpoch in the activation stamp.
    MissingSequenceEpoch(SourceIdentity),
    /// The assembled Filter failed model validation.
    Validation(EventFilterError),
}

impl Display for CompileError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySources => formatter.write_str("no FilterSources supplied"),
            Self::DuplicateIdentity(identity) => write!(
                formatter,
                "duplicate Filter identity {}/{}",
                identity.specification_id, identity.specification_version
            ),
            Self::EmptyEventKinds(identity) => write!(
                formatter,
                "FilterSource {}/{} projected zero event kinds",
                identity.specification_id, identity.specification_version
            ),
            Self::MissingSequenceEpoch(identity) => write!(
                formatter,
                "activation stamp is missing a SequenceEpoch for required source {}/{}",
                identity.monitored_application(),
                identity.source()
            ),
            Self::Validation(error) => write!(formatter, "Filter validation failed: {error}"),
        }
    }
}

impl Error for CompileError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Validation(error) => Some(error),
            _ => None,
        }
    }
}

/// A compiled Event Filter plus per-Specification width reports.
///
/// `filter` is the validated, Probe-shippable [`EventFilter`]. `widths` reports
/// actual-vs-declared width per Specification (ADR-0008 cost-class signal).
/// [`CompiledFilter::specifications`] exposes the pre-validation raw
/// `SpecificationFilter`s for inspection/tooling — the validated `EventFilter`
/// itself is opaque by design.
#[derive(Clone, Debug)]
pub struct CompiledFilter {
    /// The validated, Probe-shippable Event Filter.
    pub filter: EventFilter,
    /// Per-Specification width reports, in input order.
    pub widths: Vec<WidthReport>,
    specifications: Vec<raw_model::SpecificationFilter>,
}

impl CompiledFilter {
    /// Returns the raw `SpecificationFilter`s the compiler assembled, in input
    /// order. Membership is identical to what the validated `filter` carries
    /// (validation only canonicalizes/sorts); use this to inspect emitted
    /// `event_kinds`/`projected_attribute_keys`.
    #[must_use]
    pub fn specifications(&self) -> &[raw_model::SpecificationFilter] {
        &self.specifications
    }
}

/// Compiles a set of FilterSources into a validated Event Filter.
///
/// Pure: no I/O, no clocks, no network. Projects each source's declared
/// dependencies into a sound `SpecificationFilter`, stamps the epoch, turns on
/// baseline evidence, and validates the whole envelope through
/// [`EventFilter::try_from`]. Emits Shadow/Warming/ShipAsync/FailOpen
/// non-enforcing defaults (see module docs).
///
/// # Errors
///
/// Returns [`CompileError::EmptySources`] for an empty slice,
/// [`CompileError::DuplicateIdentity`] when two sources share an identity, and
/// [`CompileError::EmptyEventKinds`] when a source projects zero Event kinds.
/// Any model-validation failure is wrapped in [`CompileError::Validation`].
pub fn compile_filter(
    sources: &[&dyn FilterSource],
    epoch: FilterEpoch,
) -> Result<CompiledFilter, CompileError> {
    if sources.is_empty() {
        return Err(CompileError::EmptySources);
    }
    let mut specifications = Vec::with_capacity(sources.len());
    let mut widths = Vec::with_capacity(sources.len());
    let mut seen = BTreeSet::new();
    for source in sources {
        let identity = source.filter_identity();
        if !seen.insert(identity.clone()) {
            return Err(CompileError::DuplicateIdentity(identity));
        }
        let (specification, report) = compile_one(*source, epoch)?;
        specifications.push(specification);
        widths.push(report);
    }
    let raw = raw_model::EventFilter {
        epoch: Some(epoch.value()),
        specifications,
        baseline_evidence: Some(raw_model::BaselineEvidenceFilter {
            source_health: true,
            completeness_signals: true,
        }),
    };
    let compiled = raw.specifications.clone();
    let filter = EventFilter::try_from(raw).map_err(CompileError::Validation)?;
    Ok(CompiledFilter {
        filter,
        widths,
        specifications: compiled,
    })
}

/// Projects one FilterSource into its raw `SpecificationFilter` and width report.
///
/// Builds the S1/S2 unions via [`project`], rejects an empty kind set, and
/// assembles the non-enforcing defaults. See the module docs for why each
/// default validates under unpublished Warming.
fn compile_one(
    source: &dyn FilterSource,
    epoch: FilterEpoch,
) -> Result<(raw_model::SpecificationFilter, WidthReport), CompileError> {
    let identity = source.filter_identity();
    let (kinds, keys) = project(source);
    if kinds.is_empty() {
        return Err(CompileError::EmptyEventKinds(identity));
    }
    let specification = raw_model::SpecificationFilter {
        specification_id: identity.specification_id.clone(),
        specification_version: identity.specification_version.clone(),
        event_match: Some(raw_model::EventMatch {
            event_kinds: kinds
                .iter()
                .map(EventKind::as_str)
                .map(str::to_owned)
                .collect(),
            projected_attribute_keys: keys
                .iter()
                .map(AttributeKey::as_str)
                .map(str::to_owned)
                .collect(),
            delivery_mode: raw_model::DeliveryMode::ShipAsync as i32,
        }),
        fail_mode: raw_model::FailMode::Open as i32,
        latency_budget_nanoseconds: None,
        evaluation_mode: raw_model::EvaluationMode::Shadow as i32,
        readiness: raw_model::Readiness::Warming as i32,
        target_filter_epoch: Some(epoch.value()),
        required_sources: source
            .required_sources()
            .iter()
            .map(SourceIdentity::to_wire)
            .collect(),
        activation_watermarks: Vec::new(),
        enforcement_gates: None,
    };
    let report = WidthReport::new(identity, kinds.len(), keys.len(), source.width_budget());
    Ok((specification, report))
}

/// Computes the S1/S2 soundness unions for one FilterSource.
///
/// `event_kinds = { d.kind : d ∈ dependencies }` (S1, by construction).
/// `projected_attribute_keys = (⋃ d.attribute_keys) ∪ binding_attribute_keys`
/// (S2, by construction). Both are `BTreeSet`s, so the emitted `Vec`s are
/// sorted and deduped — matching the model's canonicalization.
fn project(source: &dyn FilterSource) -> (BTreeSet<EventKind>, BTreeSet<AttributeKey>) {
    let mut kinds = BTreeSet::new();
    let mut keys = BTreeSet::new();
    for dependency in source.dependencies() {
        kinds.insert(dependency.kind);
        for key in dependency.attribute_keys {
            keys.insert(key);
        }
    }
    for key in source.binding_attribute_keys() {
        keys.insert(key);
    }
    (kinds, keys)
}

/// Publication-time data the Filter compiler cannot read: an ObservationTime
/// the caller supplies, plus one SequenceEpoch per required source.
///
/// Compile stays clock-free. The caller chooses `observed_at` — it is the
/// publication instant, not a compile-time constant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivationStamp {
    /// The publication instant written onto every activation Watermark.
    pub observed_at: ObservationTime,
    /// Sequence epoch each required source is speaking in.
    pub sequence_epochs: BTreeMap<SourceIdentity, SequenceEpoch>,
}

/// Builds one activation Watermark per required source from the stamp.
///
/// Walks [`FilterSource::required_sources`] only. Does not match on Pattern
/// shape: CAT-1 and CAT-4 therefore receive a watermark per required source
/// the same way. `complete_through` is `0` — `observed_at` is the freshness
/// boundary this helper inherits.
///
/// # Errors
///
/// Returns [`CompileError::MissingSequenceEpoch`] when the stamp has no
/// SequenceEpoch for a required source. Never emits a silent empty barrier
/// for a source the stamp omitted.
pub fn activation_watermarks_for(
    source: &dyn FilterSource,
    stamp: &ActivationStamp,
) -> Result<Vec<raw_model::Watermark>, CompileError> {
    let required = source.required_sources();
    let mut watermarks = Vec::with_capacity(required.len());
    for identity in required {
        let sequence_epoch = stamp
            .sequence_epochs
            .get(&identity)
            .copied()
            .ok_or_else(|| CompileError::MissingSequenceEpoch(identity.clone()))?;
        watermarks.push(raw_model::Watermark {
            source: Some(identity.to_wire()),
            sequence_epoch: sequence_epoch.value(),
            complete_through: SequenceNumber::new(0).value(),
            observed_at: Some(stamp.observed_at.to_wire()),
        });
    }
    Ok(watermarks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sentinel_model::wire::model::v1 as raw_model;

    /// Test-only FilterSource built from raw declared parts.
    struct TestSpec {
        id: String,
        version: String,
        deps: Vec<EventDependency>,
        binding_keys: Vec<AttributeKey>,
        sources: Vec<SourceIdentity>,
        budget: FilterWidthBudget,
    }

    impl TestSpec {
        fn new(
            id: &str,
            version: &str,
            deps: Vec<EventDependency>,
            binding_keys: Vec<AttributeKey>,
            sources: Vec<SourceIdentity>,
            budget: FilterWidthBudget,
        ) -> Self {
            Self {
                id: id.to_owned(),
                version: version.to_owned(),
                deps,
                binding_keys,
                sources,
                budget,
            }
        }
    }

    impl FilterSource for TestSpec {
        fn filter_identity(&self) -> FilterIdentity {
            FilterIdentity {
                specification_id: self.id.clone(),
                specification_version: self.version.clone(),
            }
        }
        fn dependencies(&self) -> Vec<EventDependency> {
            self.deps.clone()
        }
        fn binding_attribute_keys(&self) -> Vec<AttributeKey> {
            self.binding_keys.clone()
        }
        fn required_sources(&self) -> Vec<SourceIdentity> {
            self.sources.clone()
        }
        fn width_budget(&self) -> FilterWidthBudget {
            self.budget
        }
    }

    fn attr_key(key: &str) -> AttributeKey {
        AttributeKey::new(key).expect("valid fixture key")
    }

    fn dep(kind: &str, keys: &[&str]) -> EventDependency {
        EventDependency {
            kind: EventKind::new(kind).expect("valid kind"),
            attribute_keys: keys.iter().copied().map(attr_key).collect(),
        }
    }

    fn source(app: &str, src: &str) -> SourceIdentity {
        SourceIdentity::new(
            sentinel_model::MonitoredApplicationId::new(app).expect("valid app"),
            sentinel_model::SourceId::new(src).expect("valid source"),
        )
    }

    fn epoch() -> FilterEpoch {
        FilterEpoch::new(7)
    }

    /// Extract the single SpecificationFilter emitted for one source.
    fn one_spec(compiled: &CompiledFilter) -> &raw_model::SpecificationFilter {
        assert_eq!(
            compiled.specifications().len(),
            1,
            "expected one specification"
        );
        &compiled.specifications()[0]
    }

    fn event_kinds(spec: &raw_model::SpecificationFilter) -> BTreeSet<String> {
        spec.event_match
            .as_ref()
            .expect("event_match present")
            .event_kinds
            .iter()
            .cloned()
            .collect()
    }

    fn attr_keys(spec: &raw_model::SpecificationFilter) -> BTreeSet<String> {
        spec.event_match
            .as_ref()
            .expect("event_match present")
            .projected_attribute_keys
            .iter()
            .cloned()
            .collect()
    }

    // --- Test 1: soundness_combinatorial_table_test ----------------------

    #[test]
    fn soundness_combinatorial_table_test() {
        // {1..3 deps} × {0..2 attr-keys per dep} × {shared/disjoint kinds}
        // × {0..1 binding keys} × {empty/non-empty required_sources}.
        // For every combination: compile and assert S1 + S2.
        let kind_pool = ["k.alpha", "k.beta", "k.gamma", "k.delta", "k.epsilon"];
        let key_pool = ["a.one", "a.two", "a.three", "a.four", "a.five"];
        let binding_pool = ["b.bind"];
        let src_a = source("app", "src-a");
        let src_b = source("app", "src-b");

        for dep_count in 1..=3usize {
            for keys_per_dep in 0..=2usize {
                for shared in [false, true] {
                    for binding_count in 0..=1usize {
                        for nonempty_sources in [false, true] {
                            let mut deps = Vec::new();
                            for i in 0..dep_count {
                                let kind = if shared {
                                    kind_pool[0]
                                } else {
                                    kind_pool[i % kind_pool.len()]
                                };
                                let keys: Vec<&str> = (0..keys_per_dep)
                                    .map(|j| key_pool[(i + j) % key_pool.len()])
                                    .collect();
                                deps.push(dep(kind, &keys));
                            }
                            let binding_keys: Vec<AttributeKey> = binding_pool
                                .iter()
                                .take(binding_count)
                                .copied()
                                .map(attr_key)
                                .collect();
                            let sources: Vec<SourceIdentity> = if nonempty_sources {
                                vec![src_a.clone(), src_b.clone()]
                            } else {
                                Vec::new()
                            };
                            let budget = FilterWidthBudget {
                                event_kinds: 16,
                                attribute_keys: 16,
                            };
                            let spec = TestSpec::new(
                                "comb.spec",
                                "v1",
                                deps.clone(),
                                binding_keys.clone(),
                                sources,
                                budget,
                            );
                            let compiled =
                                compile_filter(&[&spec], epoch()).expect("compile succeeds");
                            let emitted = one_spec(&compiled);
                            let emitted_kinds = event_kinds(emitted);
                            let emitted_keys = attr_keys(emitted);

                            // S1: every declared dep kind is emitted.
                            for d in &deps {
                                assert!(
                                    emitted_kinds.contains(d.kind.as_str()),
                                    "S1 violated: kind {} not emitted (deps={}, shared={})",
                                    d.kind,
                                    dep_count,
                                    shared
                                );
                            }
                            // S2: every dep attr-key ∪ binding-key is emitted.
                            for d in &deps {
                                for k in &d.attribute_keys {
                                    assert!(
                                        emitted_keys.contains(k.as_str()),
                                        "S2 violated: attr key {k} not emitted"
                                    );
                                }
                            }
                            for k in &binding_keys {
                                assert!(
                                    emitted_keys.contains(k.as_str()),
                                    "S2 violated: binding key {k} not emitted"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // --- Test 2: soundness_table_test -------------------------------------

    #[test]
    fn soundness_table_test() {
        // single dep, one key.
        let spec = TestSpec::new(
            "s.single",
            "v1",
            vec![dep("k.one", &["a.x"])],
            vec![],
            vec![],
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );
        let compiled = compile_filter(&[&spec], epoch()).unwrap();
        let emitted = one_spec(&compiled);
        assert_eq!(event_kinds(emitted), set_of(&["k.one"]));
        assert_eq!(attr_keys(emitted), set_of(&["a.x"]));

        // two deps same kind → deduped to one kind; keys unioned.
        let spec = TestSpec::new(
            "s.dedup",
            "v1",
            vec![dep("k.one", &["a.x"]), dep("k.one", &["a.y"])],
            vec![],
            vec![],
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );
        let compiled = compile_filter(&[&spec], epoch()).unwrap();
        let emitted = one_spec(&compiled);
        assert_eq!(event_kinds(emitted), set_of(&["k.one"]));
        assert_eq!(attr_keys(emitted), set_of(&["a.x", "a.y"]));

        // overlapping keys across deps → deduped.
        let spec = TestSpec::new(
            "s.overlap",
            "v1",
            vec![dep("k.one", &["a.x", "a.y"]), dep("k.two", &["a.y", "a.z"])],
            vec![],
            vec![],
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );
        let compiled = compile_filter(&[&spec], epoch()).unwrap();
        let emitted = one_spec(&compiled);
        assert_eq!(event_kinds(emitted), set_of(&["k.one", "k.two"]));
        assert_eq!(attr_keys(emitted), set_of(&["a.x", "a.y", "a.z"]));

        // empty binding keys → only dep keys.
        let spec = TestSpec::new(
            "s.nobind",
            "v1",
            vec![dep("k.one", &["a.x"])],
            vec![],
            vec![],
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );
        let compiled = compile_filter(&[&spec], epoch()).unwrap();
        let emitted = one_spec(&compiled);
        assert_eq!(attr_keys(emitted), set_of(&["a.x"]));

        // dep with required attrs + a binding key → union.
        let spec = TestSpec::new(
            "s.bind",
            "v1",
            vec![dep("k.one", &["a.x"])],
            vec![attr_key("b.bind")],
            vec![],
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );

        let compiled = compile_filter(&[&spec], epoch()).unwrap();
        let emitted = one_spec(&compiled);
        assert_eq!(attr_keys(emitted), set_of(&["a.x", "b.bind"]));
    }

    fn set_of(values: &[&str]) -> BTreeSet<String> {
        values.iter().copied().map(str::to_owned).collect()
    }

    // --- Test 3: width_over_budget_test -----------------------------------

    #[test]
    fn width_over_budget_test() {
        // actual: 2 kinds + 2 keys = 4; declared: 1 + 1 = 2 → over budget.
        let spec = TestSpec::new(
            "s.over",
            "v1",
            vec![dep("k.one", &["a.x"]), dep("k.two", &["a.y"])],
            vec![],
            vec![],
            FilterWidthBudget {
                event_kinds: 1,
                attribute_keys: 1,
            },
        );
        let compiled = compile_filter(&[&spec], epoch()).expect("compile succeeds");
        let report = &compiled.widths[0];
        assert!(report.over_budget);
        assert_eq!(report.actual_width, 4);
        assert_eq!(report.declared_width, 2);
        // NOT truncated: every declared kind/key still present.
        let emitted = one_spec(&compiled);
        assert!(event_kinds(emitted).contains("k.one"));
        assert!(event_kinds(emitted).contains("k.two"));
        assert!(attr_keys(emitted).contains("a.x"));
        assert!(attr_keys(emitted).contains("a.y"));
    }

    // --- Test 4: width_under_and_exact_test -------------------------------

    #[test]
    fn width_under_and_exact_test() {
        // under: actual 1+1=2, declared 4+4=8.
        let spec = TestSpec::new(
            "s.under",
            "v1",
            vec![dep("k.one", &["a.x"])],
            vec![],
            vec![],
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );
        let compiled = compile_filter(&[&spec], epoch()).unwrap();
        assert!(!compiled.widths[0].over_budget);

        // exact: actual 1+1=2, declared 1+1=2.
        let spec = TestSpec::new(
            "s.exact",
            "v1",
            vec![dep("k.one", &["a.x"])],
            vec![],
            vec![],
            FilterWidthBudget {
                event_kinds: 1,
                attribute_keys: 1,
            },
        );
        let compiled = compile_filter(&[&spec], epoch()).unwrap();
        assert!(!compiled.widths[0].over_budget);
        assert_eq!(
            compiled.widths[0].actual_width,
            compiled.widths[0].declared_width
        );
    }

    // --- Test 5: width_never_truncates_test -------------------------------

    #[test]
    fn width_never_truncates_test() {
        // Deliberately tight budget; every declared kind/key must survive.
        let spec = TestSpec::new(
            "s.notrunc",
            "v1",
            vec![dep("k.one", &["a.x", "a.y"]), dep("k.two", &["a.z"])],
            vec![attr_key("b.bind")],
            vec![],
            FilterWidthBudget {
                event_kinds: 1,
                attribute_keys: 1,
            },
        );
        let compiled = compile_filter(&[&spec], epoch()).expect("compile succeeds");
        assert!(compiled.widths[0].over_budget);
        let emitted = one_spec(&compiled);
        let kinds = event_kinds(emitted);
        let keys = attr_keys(emitted);
        for k in &["k.one", "k.two"] {
            assert!(kinds.contains(*k), "kind {k} truncated");
        }
        for k in &["a.x", "a.y", "a.z", "b.bind"] {
            assert!(keys.contains(*k), "key {k} truncated");
        }
    }

    // --- Test 6: validation_round_trip_test -------------------------------

    #[test]
    fn validation_round_trip_test() {
        let spec = TestSpec::new(
            "s.roundtrip",
            "v1",
            vec![dep("k.one", &["a.x"])],
            vec![],
            vec![source("app", "src")],
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );
        let compiled = compile_filter(&[&spec], epoch()).expect("compile succeeds");
        // The compiler already validated via EventFilter::try_from. Rebuild the
        // raw envelope from the emitted specs and assert it re-validates.
        let raw = raw_model::EventFilter {
            epoch: Some(epoch().value()),
            specifications: compiled.specifications().to_vec(),
            baseline_evidence: Some(raw_model::BaselineEvidenceFilter {
                source_health: true,
                completeness_signals: true,
            }),
        };
        EventFilter::try_from(raw).expect("emitted filter re-validates");
        // Also assert the validated filter encodes.
        assert!(!compiled.filter.encode_to_vec().is_empty());
    }

    // --- Test 7: rejection cases ------------------------------------------

    #[test]
    fn empty_sources_rejected() {
        let err = compile_filter(&[], epoch()).unwrap_err();
        assert_eq!(err, CompileError::EmptySources);
    }

    #[test]
    fn duplicate_identity_rejected() {
        let spec = TestSpec::new(
            "s.dup",
            "v1",
            vec![dep("k.one", &["a.x"])],
            vec![],
            vec![],
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );
        let err = compile_filter(&[&spec, &spec], epoch()).unwrap_err();
        match err {
            CompileError::DuplicateIdentity(id) => {
                assert_eq!(id.specification_id, "s.dup");
                assert_eq!(id.specification_version, "v1");
            }
            other => panic!("expected DuplicateIdentity, got {other:?}"),
        }
    }

    #[test]
    fn empty_event_kinds_rejected() {
        let spec = TestSpec::new(
            "s.empty",
            "v1",
            vec![],
            vec![attr_key("b.bind")],
            vec![],
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );
        let err = compile_filter(&[&spec], epoch()).unwrap_err();
        match err {
            CompileError::EmptyEventKinds(id) => {
                assert_eq!(id.specification_id, "s.empty");
            }
            other => panic!("expected EmptyEventKinds, got {other:?}"),
        }
    }

    // --- Test 8: defaults_are_shadow_warming_shipasync_failopen_test -------

    #[test]
    fn defaults_are_shadow_warming_shipasync_failopen_test() {
        let spec = TestSpec::new(
            "s.defaults",
            "v1",
            vec![dep("k.one", &["a.x"])],
            vec![],
            vec![source("app", "src")],
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );
        let compiled = compile_filter(&[&spec], epoch()).expect("compile succeeds");
        let emitted = one_spec(&compiled);
        assert_eq!(
            emitted.evaluation_mode,
            raw_model::EvaluationMode::Shadow as i32
        );
        assert_eq!(emitted.readiness, raw_model::Readiness::Warming as i32);
        assert_eq!(
            emitted.event_match.as_ref().unwrap().delivery_mode,
            raw_model::DeliveryMode::ShipAsync as i32
        );
        assert_eq!(emitted.fail_mode, raw_model::FailMode::Open as i32);
        assert_eq!(emitted.latency_budget_nanoseconds, None);
        assert_eq!(emitted.enforcement_gates, None);
        assert!(emitted.activation_watermarks.is_empty());
        assert_eq!(emitted.target_filter_epoch, Some(epoch().value()));
    }

    // --- Test 9: baseline_evidence_always_on_test -------------------------

    #[test]
    fn baseline_evidence_always_on_test() {
        let spec = TestSpec::new(
            "s.baseline",
            "v1",
            vec![dep("k.one", &["a.x"])],
            vec![],
            vec![],
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );
        let compiled = compile_filter(&[&spec], epoch()).expect("compile succeeds");
        // Re-validate from raw to confirm baseline is on and accepted.
        let raw = raw_model::EventFilter {
            epoch: Some(epoch().value()),
            specifications: compiled.specifications().to_vec(),
            baseline_evidence: Some(raw_model::BaselineEvidenceFilter {
                source_health: true,
                completeness_signals: true,
            }),
        };
        EventFilter::try_from(raw).expect("baseline-on filter validates");
    }

    // --- Test 10: required_sources_preserved_test -------------------------

    #[test]
    fn required_sources_preserved_test() {
        let srcs = vec![source("app-one", "src-a"), source("app-two", "src-b")];
        let spec = TestSpec::new(
            "s.sources",
            "v1",
            vec![dep("k.one", &["a.x"])],
            vec![],
            srcs.clone(),
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );
        let compiled = compile_filter(&[&spec], epoch()).expect("compile succeeds");
        let emitted = one_spec(&compiled);
        let emitted_srcs: BTreeSet<(String, String)> = emitted
            .required_sources
            .iter()
            .map(|s| (s.monitored_application_id.clone(), s.source_id.clone()))
            .collect();
        let want: BTreeSet<(String, String)> = srcs
            .iter()
            .map(|s| {
                (
                    s.monitored_application().to_string(),
                    s.source().to_string(),
                )
            })
            .collect();
        assert_eq!(emitted_srcs, want);
    }

    // --- Test 11: epoch_stamped_test --------------------------------------

    #[test]
    fn epoch_stamped_test() {
        let spec = TestSpec::new(
            "s.epoch",
            "v1",
            vec![dep("k.one", &["a.x"])],
            vec![],
            vec![],
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );
        let e = FilterEpoch::new(42);
        let compiled = compile_filter(&[&spec], e).expect("compile succeeds");
        let emitted = one_spec(&compiled);
        assert_eq!(emitted.target_filter_epoch, Some(42));
    }

    // --- Test 12: multiple_specs_independent_filters_test ----------------

    #[test]
    fn multiple_specs_independent_filters_test() {
        let spec_a = TestSpec::new(
            "s.a",
            "v1",
            vec![dep("k.one", &["a.x"])],
            vec![],
            vec![],
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );
        let spec_b = TestSpec::new(
            "s.b",
            "v2",
            vec![dep("k.two", &["a.y"]), dep("k.three", &["a.z"])],
            vec![attr_key("b.bind")],
            vec![source("app", "src")],
            FilterWidthBudget {
                event_kinds: 1,
                attribute_keys: 1,
            },
        );
        let compiled = compile_filter(&[&spec_a, &spec_b], epoch()).expect("compile succeeds");
        assert_eq!(compiled.specifications().len(), 2);
        assert_eq!(compiled.widths.len(), 2);
        // Identities unique and ordered as input.
        assert_eq!(compiled.specifications()[0].specification_id, "s.a");
        assert_eq!(compiled.specifications()[1].specification_id, "s.b");
        // Independent widths: a under, b over.
        assert!(!compiled.widths[0].over_budget);
        assert!(compiled.widths[1].over_budget);
        // Independent event_kinds.
        assert_eq!(
            event_kinds(&compiled.specifications()[0]),
            set_of(&["k.one"])
        );
        assert_eq!(
            event_kinds(&compiled.specifications()[1]),
            set_of(&["k.two", "k.three"])
        );
    }

    // --- Test 13: empty_projected_keys_emits_empty_test ------------------

    #[test]
    fn empty_projected_keys_emits_empty_test() {
        // Deps with no attr keys AND no binding keys → empty projected_attribute_keys.
        // LOAD-BEARING: the consumer treats empty as retain-all, so the compiler
        // must emit empty (never a sentinel). Pinning this prevents a future
        // change from filling projected_attribute_keys and silently breaking
        // retain-all.
        let spec = TestSpec::new(
            "s.emptykeys",
            "v1",
            vec![dep("k.one", &[])],
            vec![],
            vec![],
            FilterWidthBudget {
                event_kinds: 4,
                attribute_keys: 4,
            },
        );
        let compiled = compile_filter(&[&spec], epoch()).expect("compile succeeds");
        let emitted = one_spec(&compiled);
        assert!(emitted
            .event_match
            .as_ref()
            .unwrap()
            .projected_attribute_keys
            .is_empty());
        // event_kinds still present.
        assert_eq!(event_kinds(emitted), set_of(&["k.one"]));
    }
}
