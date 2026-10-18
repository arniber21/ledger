//! Fixture loading, the testbed Specification, and every named mutation helper.
//!
//! # Where the SagaShop-shaped Specification lives
//!
//! [`ship_requires_prior_charge`] is a **testbed fixture**, not a Catalogue
//! entry, so it lives here rather than in `sentinel_core`. Putting a
//! SagaShop-shaped constructor in a production crate would make issue #14's
//! first task deleting it.
//!
//! # Mutation-helper contract
//!
//! Derived fixtures are produced in-test by named mutations of a loaded real
//! capture, never checked in pre-mutated, so the diff from the real artifact is
//! explicit and cannot silently drift.
//!
//! Two contracts are load bearing, and getting either wrong makes a test pass
//! for the wrong reason:
//!
//! 1. **Deletion renumbers.** `shared/capture.ts` requires `sourceSequence` to
//!    be per-service and **contiguous from 1**, and the Adapter maps it onto a
//!    `DeliverySlot`. So deleting a span from a service that has other spans
//!    breaks contiguity and fires the evidence gate's **G2** disjunct. Every
//!    deletion helper here therefore renumbers the affected service's
//!    `sourceSequence` to stay contiguous — with exactly one exception,
//!    [`delete_span_leaving_sequence_gap`], which pulls that lever *deliberately*
//!    and is the only helper the sequence-gap test may use. Without this, the
//!    absent-anchor-Event and dangling-edge tests would both be
//!    `Inconclusive(prefix)` via G2 rather than via the gate they exist to
//!    exercise, and those gates would ship unverified.
//!
//! 2. **Dangling edges are made without touching sequences.**
//!    [`retarget_logical_parent`] repoints a span's `logicalParentSpanId` at an
//!    identifier no span carries, which produces an unresolved causal
//!    predecessor with **zero** sequence disturbance — isolating the gate's G4
//!    disjunct exactly. The Adapter cooperates by treating an unresolved
//!    `logicalParentSpanId` as a dangling causal edge rather than a parse error:
//!    ADR-0019 explicitly wants missing parents handled as evidence.
//!
//! A third, narrower contract: [`duplicate_span`] assigns the copy a **fresh**
//! `sourceSequence` (`max + 1` for that service) and the completeness signal is
//! recomputed from it, because reusing an existing `sourceSequence` produces a
//! `SlotConflict` — a different test's subject — instead of the second trigger
//! the mixed-outcome test exists to construct.

#![allow(dead_code)] // Helpers are shared across test binaries; not all are used by each.
#![allow(clippy::doc_markdown)] // Product names read as prose here.

/// CAT-4 fixtures built from Events, shared by `taint_evaluator.rs` and
/// `subsumption_cross_check.rs`. A submodule because its names (`source`,
/// `BINDING`, …) are CAT-4's own and would collide with the SagaShop ones here.
pub mod taint;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use prost::Message;
use sentinel_model::wire::model::v1 as raw_model;
use sentinel_model::wire::probe::v1 as raw_probe;

use sentinel_compile::catalogue::{
    instantiate, EvidenceBinding, InstanceError, ParameterValue, PatternId, PatternInstance,
    ProducerContract, SelectorValue,
};
use sentinel_compile::FilterIdentity;
use sentinel_core::evaluator::{Evaluator, Input};
use sentinel_core::facts::{ColdGuardId, FactRequest, FactResponse, GuardConclusion};
use sentinel_core::slice::{Binding, RetirementCause, RetirementSignal, ScopeHorizon};
use sentinel_core::specification::{
    AttributeKey, EventSelector, PrecedenceSpecification, RequiredSource, RetirementPolicy,
    SpecificationId, SpecificationIdentity, SpecificationVersion,
};
use sentinel_core::verdict::BindingVerdict;
use sentinel_core::CoreError;
use sentinel_ingest::capture::{Capture, CaptureAttribute, CapturedSpan};
use sentinel_ingest::evidence::ObservedEvidence;
use sentinel_ingest::sagashop::SagaShopAdapter;
use sentinel_model::{
    ActivationBarrier, ActivationWatermark, AttributeValue, DecideRequest, Event, EventFilter,
    EventKind, FilterEpoch, MonitoredApplicationId, ObservationTime, SchemaVersion,
    SentinelClockId, SequenceEpoch, SequenceNumber, SourceEvidence, SourceId, SourceIdentity,
    SourceTier,
};

/// The order service, a contributing-tier source inside the application.
pub const ORDER_SERVICE: &str = "sagashop-order";
/// The payment service, an anchor-tier source the order service calls through.
pub const PAYMENT_SERVICE: &str = "sagashop-payment";
/// The shipping service, an anchor-tier source not required by this Specification.
pub const SHIPPING_SERVICE: &str = "sagashop-shipping";

/// The `sagashop.event` value of the span that ships an order.
pub const ORDER_SHIP: &str = "order.ship";
/// The `sagashop.event` value of the span that charges an order.
pub const ORDER_CHARGE: &str = "order.charge";
/// The `sagashop.event` value of the payment service's charge span.
pub const PAYMENT_CHARGE: &str = "payment.charge";
/// The `sagashop.event` value of the shipping service's schedule span.
pub const SHIPPING_SCHEDULE: &str = "shipping.schedule";
/// The inventory service, an anchor-tier source the order service calls through.
pub const INVENTORY_SERVICE: &str = "sagashop-inventory";
/// The `sagashop.event` value of the span that creates an order.
pub const ORDER_CREATE: &str = "order.create";
/// The `sagashop.event` value of the span that fulfills an order.
pub const ORDER_FULFILL: &str = "order.fulfill";
/// The `sagashop.event` value of the span that cancels an order.
pub const ORDER_CANCEL: &str = "order.cancel";
/// The `sagashop.event` value of the inventory service's reserve span.
pub const INVENTORY_RESERVE: &str = "inventory.reserve";

/// Returns the vendored fixture directory.
#[must_use]
pub fn testdata_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("testdata")
        .join("sagashop")
}

/// Returns the path of one vendored capture.
#[must_use]
pub fn capture_path(scenario: &str) -> PathBuf {
    testdata_dir().join(format!("ts-{scenario}.json"))
}

/// Loads and structurally validates one vendored, unmodified capture.
///
/// # Panics
///
/// Panics when the fixture is absent or fails structural validation, which
/// would mean the vendored artifact no longer matches its recorded provenance.
#[must_use]
pub fn load_capture(scenario: &str) -> Capture {
    let path = capture_path(scenario);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("vendored capture {} is readable: {error}", path.display()));
    Capture::from_json(&bytes)
        .unwrap_or_else(|error| panic!("vendored capture {} decodes: {error}", path.display()))
}

/// Reads SagaShop's frozen property expectation for one scenario and property.
///
/// The expectation is read from the vendored oracle artifact rather than
/// hardcoded, which is what makes "the Verdict matches SagaShop's
/// oracle-declared expectation" literal rather than aspirational.
///
/// # Panics
///
/// Panics when the matrix is absent or does not carry the requested cell.
#[must_use]
pub fn frozen_property_expectation(scenario: &str, property: &str) -> String {
    let path = testdata_dir().join("scenario-property-matrix.json");
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("frozen matrix is readable: {error}"));
    // The matrix is a tiny frozen artifact with a flat, quoted shape. Reading
    // the one cell by hand keeps `core` free of a JSON dependency it would
    // otherwise take on only for tests.
    let scenarios = text
        .split_once("\"scenarios\"")
        .expect("frozen matrix declares scenarios")
        .1;
    let row = scenarios
        .split_once(&format!("\"{scenario}\""))
        .unwrap_or_else(|| panic!("frozen matrix declares scenario {scenario}"))
        .1;
    let row = &row[..row.find('}').expect("scenario row is closed")];
    let cell = row
        .split_once(&format!("\"{property}\""))
        .unwrap_or_else(|| panic!("frozen matrix declares {scenario}.{property}"))
        .1;
    let cell = cell
        .split_once(':')
        .expect("property cell has a value")
        .1
        .trim_start();
    let cell = cell
        .strip_prefix('"')
        .expect("property cell holds a quoted value");
    cell[..cell.find('"').expect("property cell is closed")].to_owned()
}

/// Returns the canonical source identity for one SagaShop service.
///
/// # Panics
///
/// Panics for an empty service name, which the capture decoder rejects.
#[must_use]
pub fn source(service: &str) -> SourceIdentity {
    SourceIdentity::new(
        MonitoredApplicationId::new(service).expect("service name is not empty"),
        SourceId::new(sentinel_ingest::sagashop::SOURCE_ID).expect("constant source id"),
    )
}

/// The testbed Specification: SagaShop property P4, `ship-requires-prior-charge`.
///
/// A ship may only happen for an order that was already successfully charged.
/// The requirement Event comes from `sagashop-payment` and the trigger from
/// `sagashop-order`, so this is genuinely cross-system rather than decidable
/// from one service's own state machine — which is the point, per ADR-0019's
/// demand for Specifications "beyond local guardrails and ordinary trace
/// queries", and which is what lets an authoritative Verdict exist at all
/// (the requirement Event is anchor tier).
///
/// # Panics
///
/// Panics only if a constant identifier in this function is empty.
#[must_use]
pub fn ship_requires_prior_charge() -> PrecedenceSpecification {
    let mut successful_charge = BTreeMap::new();
    successful_charge.insert(
        AttributeKey::new("sagashop.payment.status").expect("fixture"),
        AttributeValue::String("success".to_owned()),
    );

    let mut required = BTreeSet::new();
    required.insert(RequiredSource::new(
        source(ORDER_SERVICE),
        SourceTier::Contributing,
    ));
    required.insert(RequiredSource::new(
        source(PAYMENT_SERVICE),
        SourceTier::Anchor,
    ));

    PrecedenceSpecification::new(
        SpecificationIdentity::new(
            SpecificationId::new("sagashop.p4.ship-requires-prior-charge").expect("fixture"),
            SpecificationVersion::new("v1").expect("fixture"),
        ),
        AttributeKey::new("sagashop.order.id").expect("fixture"),
        EventSelector::new(
            EventKind::new("sagashop.order.ship.requested").expect("constant kind"),
            BTreeMap::new(),
        ),
        EventSelector::new(
            EventKind::new("sagashop.payment.charge.completed").expect("constant kind"),
            successful_charge,
        ),
        required,
        SchemaVersion::new("sagashop-contract-v1").expect("constant schema version"),
    )
}

/// The P4 Specification with its required-source set emptied.
///
/// A Specification that requires nothing is a defect in the Specification, not
/// in the evidence: Evidence Coverage over an empty required set is vacuously
/// empty, and reporting that as an evidence failure would name a deployment
/// fault with no sources to instrument.
/// `PrecedenceSpecification::new` is infallible — rejecting a vacuous
/// Specification belongs at Promotion, in `sentinel-compile` — so this shape is
/// constructible and the evaluator has to answer for it by name.
#[must_use]
pub fn ship_requires_prior_charge_without_required_sources() -> PrecedenceSpecification {
    let complete = ship_requires_prior_charge();
    PrecedenceSpecification::new(
        complete.identity().clone(),
        complete.binding_attribute().clone(),
        complete.trigger().clone(),
        complete.requirement().clone(),
        BTreeSet::new(),
        complete.schema_version().clone(),
    )
}

/// The testbed Specification for P6: `charge-requires-prior-reserve`.
///
/// A charge may only happen for an order whose inventory was already reserved.
/// The requirement Event comes from `sagashop-inventory` (anchor tier) and the
/// trigger from `sagashop-order` (contributing), making this cross-system.
///
/// The requirement selector carries `sagashop.inventory.reserved = true` so that
/// an injected violation via `set_span_attribute(reserve,
/// "sagashop.inventory.reserved", "False")` empties the matching requirement set
/// while the reserve Event stays present as an anchor candidate — producing an
/// authoritative Violate rather than the Inconclusive the anchor-downgrade rule
/// (precedence.rs §4.3) would otherwise impose when the requirement Event is
/// simply deleted.
#[must_use]
pub fn charge_requires_prior_reserve() -> PrecedenceSpecification {
    let mut successful_reserve = BTreeMap::new();
    successful_reserve.insert(
        AttributeKey::new("sagashop.inventory.reserved").expect("fixture"),
        AttributeValue::Bool(true),
    );

    let mut required = BTreeSet::new();
    required.insert(RequiredSource::new(
        source(ORDER_SERVICE),
        SourceTier::Contributing,
    ));
    required.insert(RequiredSource::new(
        source(INVENTORY_SERVICE),
        SourceTier::Anchor,
    ));

    PrecedenceSpecification::new(
        SpecificationIdentity::new(
            SpecificationId::new("sagashop.p6.charge-requires-prior-reserve").expect("fixture"),
            SpecificationVersion::new("v1").expect("fixture"),
        ),
        AttributeKey::new("sagashop.order.id").expect("fixture"),
        EventSelector::new(
            EventKind::new("sagashop.order.charge.requested").expect("constant kind"),
            BTreeMap::new(),
        ),
        EventSelector::new(
            EventKind::new("sagashop.inventory.reserve.completed").expect("constant kind"),
            successful_reserve,
        ),
        required,
        SchemaVersion::new("sagashop-contract-v1").expect("constant schema version"),
    )
}

/// The testbed Specification for P8: `fulfill-requires-prior-charge`.
///
/// A fulfillment may only happen for an order that was already successfully
/// charged. Same cross-system shape as P4 (requirement from `sagashop-payment`,
/// anchor tier) but with `order.fulfill.requested` as the trigger.
#[must_use]
pub fn fulfill_requires_prior_charge() -> PrecedenceSpecification {
    let mut successful_charge = BTreeMap::new();
    successful_charge.insert(
        AttributeKey::new("sagashop.payment.status").expect("fixture"),
        AttributeValue::String("success".to_owned()),
    );

    let mut required = BTreeSet::new();
    required.insert(RequiredSource::new(
        source(ORDER_SERVICE),
        SourceTier::Contributing,
    ));
    required.insert(RequiredSource::new(
        source(PAYMENT_SERVICE),
        SourceTier::Anchor,
    ));

    PrecedenceSpecification::new(
        SpecificationIdentity::new(
            SpecificationId::new("sagashop.p8.fulfill-requires-prior-charge").expect("fixture"),
            SpecificationVersion::new("v1").expect("fixture"),
        ),
        AttributeKey::new("sagashop.order.id").expect("fixture"),
        EventSelector::new(
            EventKind::new("sagashop.order.fulfill.requested").expect("constant kind"),
            BTreeMap::new(),
        ),
        EventSelector::new(
            EventKind::new("sagashop.payment.charge.completed").expect("constant kind"),
            successful_charge,
        ),
        required,
        SchemaVersion::new("sagashop-contract-v1").expect("constant schema version"),
    )
}

// ---------------------------------------------------------------------------
// Issue #14a: the same properties, built from the Pattern catalogue
// ---------------------------------------------------------------------------

/// The producer schema contract every SagaShop Specification is written against.
///
/// A producer-contract fact, not a Pattern parameter: it says which version of
/// the Event schema the selectors below are written against, and it is the same
/// for every property. `instantiate` takes it in `ProducerContract`.
pub const SAGASHOP_SCHEMA_VERSION: &str = "sagashop-contract-v1";

/// The Event attribute whose value names a Binding in every SagaShop property.
///
/// Every P1-P8 property is quantified per order, so all four CAT-1 instances
/// below bind this same key. It is a named constant because
/// `catalogue_grounding.rs` also needs to perturb it: a Binding attribute that
/// no Event carries produces no Bindings, no Verdicts and — before the
/// non-vacuity guard there — a green test.
pub const SAGASHOP_BINDING_ATTRIBUTE: &str = "sagashop.order.id";

/// Builds one CAT-1 instance from the catalogue.
///
/// The evidence bindings carry what the **deployment** asserts, read from the
/// Adapter rather than chosen here: `source_tier` decides which services are
/// anchor tier, and `effective_capabilities` is what a recorded SagaShop capture
/// actually supports. That is the point of the split ADR-0022 draws — the
/// Pattern declares the tier it needs, the deployment reports the tier it has,
/// and `PatternInstance::validate` compares them.
fn cat1(
    specification_id: &str,
    trigger: (&str, &str),
    requirement: (&str, &[(&str, AttributeValue)]),
    requirement_kind: &str,
) -> PatternInstance {
    cat1_bound_to(
        AttributeKey::new(SAGASHOP_BINDING_ATTRIBUTE).expect("fixture binding"),
        specification_id,
        trigger,
        requirement,
        requirement_kind,
    )
    .expect("the SagaShop bindings satisfy CAT-1's declared parameters and roles")
}

/// Builds one CAT-1 instance with the Binding attribute supplied rather than
/// assumed, and hands back whatever `instantiate` decided.
///
/// Two callers want different halves of this. [`cat1`] wants the instance and
/// treats a refusal as a broken fixture. `catalogue_grounding.rs`'s vacuity
/// tests want a *well-formed but wrong* key to instantiate cleanly, because no
/// value check can tell `sagashop.order.i` from a real attribute nobody has
/// emitted yet. Blank names never reach here: [`AttributeKey::new`] refuses
/// them before a [`ParameterValue`] exists (issue #203).
///
/// # Errors
///
/// Returns whatever `sentinel_compile::catalogue::instantiate` returns.
pub fn cat1_bound_to(
    binding_attribute: AttributeKey,
    specification_id: &str,
    trigger: (&str, &str),
    requirement: (&str, &[(&str, AttributeValue)]),
    requirement_kind: &str,
) -> Result<PatternInstance, InstanceError> {
    let (trigger_kind, trigger_service) = trigger;
    let (requirement_service, requirement_attributes) = requirement;

    let arguments = BTreeMap::from([
        (
            "binding_attribute".to_owned(),
            ParameterValue::AttributeKey(binding_attribute),
        ),
        (
            "trigger".to_owned(),
            ParameterValue::Selector(SelectorValue::new(
                EventKind::new(trigger_kind).expect("constant kind"),
                BTreeMap::new(),
            )),
        ),
        (
            "requirement".to_owned(),
            ParameterValue::Selector(SelectorValue::new(
                EventKind::new(requirement_kind).expect("constant kind"),
                requirement_attributes
                    .iter()
                    .map(|(key, value)| {
                        (
                            AttributeKey::new(*key).expect("fixture attribute key"),
                            value.clone(),
                        )
                    })
                    .collect(),
            )),
        ),
    ]);

    let evidence = BTreeMap::from([
        (
            "trigger_source".to_owned(),
            EvidenceBinding::new(
                source(trigger_service),
                sentinel_ingest::sagashop::source_tier(trigger_service),
                sentinel_ingest::sagashop::effective_capabilities(),
            ),
        ),
        (
            "requirement_source".to_owned(),
            EvidenceBinding::new(
                source(requirement_service),
                sentinel_ingest::sagashop::source_tier(requirement_service),
                sentinel_ingest::sagashop::effective_capabilities(),
            ),
        ),
    ]);

    instantiate(
        PatternId::PrecedenceApproval,
        FilterIdentity {
            specification_id: specification_id.to_owned(),
            specification_version: "v1".to_owned(),
        },
        ProducerContract::new(
            SchemaVersion::new(SAGASHOP_SCHEMA_VERSION).expect("constant schema version"),
        ),
        arguments,
        evidence,
    )
}

/// The requirement selector shared by P4 and P8: a successful payment charge.
fn successful_charge() -> Vec<(&'static str, AttributeValue)> {
    vec![(
        "sagashop.payment.status",
        AttributeValue::String("success".to_owned()),
    )]
}

/// SagaShop P4 built as an instantiation of CAT-1 `precedence.approval`.
#[must_use]
pub fn cat1_ship_requires_prior_charge() -> PatternInstance {
    cat1(
        "sagashop.p4.ship-requires-prior-charge",
        ("sagashop.order.ship.requested", ORDER_SERVICE),
        (PAYMENT_SERVICE, &successful_charge()),
        "sagashop.payment.charge.completed",
    )
}

/// SagaShop P4 with the Binding attribute replaced by whatever is passed.
///
/// The one axis `catalogue_grounding.rs`'s vacuity tests vary. Everything else —
/// selectors, evidence roles, tiers, schema version — is P4 exactly, so a
/// difference in the Verdict set is attributable to the Binding attribute alone.
///
/// # Errors
///
/// Returns whatever `sentinel_compile::catalogue::instantiate` returns.
pub fn cat1_ship_requires_prior_charge_bound_to(
    binding_attribute: AttributeKey,
) -> Result<PatternInstance, InstanceError> {
    cat1_bound_to(
        binding_attribute,
        "sagashop.p4.ship-requires-prior-charge",
        ("sagashop.order.ship.requested", ORDER_SERVICE),
        (PAYMENT_SERVICE, &successful_charge()),
        "sagashop.payment.charge.completed",
    )
}

/// SagaShop P4 with the trigger Event kind replaced by whatever is passed.
///
/// The exact mirror of [`cat1_ship_requires_prior_charge_bound_to`], varying the
/// other axis. Issue #144's defect is a *trigger* selector that matches nothing:
/// unlike a wrong Binding attribute, it leaves the Binding count untouched, so
/// the discriminator #14a shipped cannot see it. Everything else here —
/// the Binding attribute, the requirement selector, the evidence roles, the
/// tiers, the schema version — is P4 exactly, so a difference in what the
/// Specification measures is attributable to the trigger kind alone.
///
/// # Errors
///
/// Returns whatever `sentinel_compile::catalogue::instantiate` returns.
pub fn cat1_ship_requires_prior_charge_triggered_by(
    trigger_kind: &str,
) -> Result<PatternInstance, InstanceError> {
    cat1_bound_to(
        AttributeKey::new(SAGASHOP_BINDING_ATTRIBUTE).expect("fixture binding"),
        "sagashop.p4.ship-requires-prior-charge",
        (trigger_kind, ORDER_SERVICE),
        (PAYMENT_SERVICE, &successful_charge()),
        "sagashop.payment.charge.completed",
    )
}

/// SagaShop P6 built as an instantiation of CAT-1 `precedence.approval`.
#[must_use]
pub fn cat1_charge_requires_prior_reserve() -> PatternInstance {
    cat1(
        "sagashop.p6.charge-requires-prior-reserve",
        ("sagashop.order.charge.requested", ORDER_SERVICE),
        (
            INVENTORY_SERVICE,
            &[("sagashop.inventory.reserved", AttributeValue::Bool(true))],
        ),
        "sagashop.inventory.reserve.completed",
    )
}

/// SagaShop P8 built as an instantiation of CAT-1 `precedence.approval`.
#[must_use]
pub fn cat1_fulfill_requires_prior_charge() -> PatternInstance {
    cat1(
        "sagashop.p8.fulfill-requires-prior-charge",
        ("sagashop.order.fulfill.requested", ORDER_SERVICE),
        (PAYMENT_SERVICE, &successful_charge()),
        "sagashop.payment.charge.completed",
    )
}

/// SagaShop P7 built as an instantiation of CAT-1, with a tier shortfall.
///
/// P7 is "reserve requires prior order create". Its requirement Event comes from
/// `sagashop-order`, which the Adapter places at **contributing** tier because it
/// is inside the Monitored Application. CAT-1 declares that its
/// `requirement_source` needs anchor tier, so this instantiation carries an
/// `InstanceWarning::TierShortfall` — the mechanical form of the English caveat
/// in `core/tests/e2e_oracle.rs`, which is why P7 is an oracle check there and
/// not a Core-evaluated Specification.
///
/// There is no hand-written fixture for P7 to be compared against, deliberately:
/// the point of this instance is the warning, not a Verdict.
#[must_use]
pub fn cat1_reserve_requires_prior_create() -> PatternInstance {
    cat1(
        "sagashop.p7.reserve-requires-prior-create",
        ("sagashop.inventory.reserve.requested", INVENTORY_SERVICE),
        (ORDER_SERVICE, &[]),
        "sagashop.order.create.requested",
    )
}

/// Returns the three catalogue-built Specifications that have hand-written
/// counterparts, paired with the property each expresses.
#[must_use]
pub fn all_cat1_instances() -> Vec<(&'static str, PatternInstance)> {
    vec![
        ("P4", cat1_ship_requires_prior_charge()),
        ("P6", cat1_charge_requires_prior_reserve()),
        ("P8", cat1_fulfill_requires_prior_charge()),
    ]
}

/// Returns all three precedence Specifications (P4, P6, P8) with their property IDs.
#[must_use]
pub fn all_precedence_specifications() -> Vec<(&'static str, PrecedenceSpecification)> {
    vec![
        ("P4", ship_requires_prior_charge()),
        ("P6", charge_requires_prior_reserve()),
        ("P8", fulfill_requires_prior_charge()),
    ]
}

/// Adapts a capture with the synchronous-call-graph premise asserted.
///
/// # Panics
///
/// Panics when adaptation fails, which for a structurally valid capture means a
/// duplicate `sourceSequence` or evidence the model rejects.
#[must_use]
pub fn adapt(capture: &Capture) -> ObservedEvidence {
    SagaShopAdapter::with_synchronous_call_graph()
        .adapt(capture)
        .expect("capture adapts into canonical Events")
}

/// Loads and adapts one unmodified vendored capture.
#[must_use]
pub fn adapted(scenario: &str) -> ObservedEvidence {
    adapt(&load_capture(scenario))
}

/// Drives the Core's batch path over one Adapter batch.
///
/// `ObservedEvidence` is an Adapter *batch handoff* shape and deliberately
/// stayed in `sentinel_ingest` when issue #12 moved `SourceEvidence` and
/// `SequenceGap` into `sentinel_model`. `sentinel-core` therefore links
/// `sentinel-ingest` only as a dev-dependency, and unpacking the batch into the
/// `(events, sources)` pair `sentinel_core::evaluate` takes is the Adapter
/// boundary's job rather than the Core's.
///
/// # Errors
///
/// Returns whatever `sentinel_core::evaluate` returns.
pub fn evaluate(
    specification: &PrecedenceSpecification,
    evidence: &ObservedEvidence,
) -> Result<Vec<BindingVerdict>, CoreError> {
    sentinel_core::evaluate(
        Evaluator::new(specification.clone()),
        evidence.events(),
        evidence.sources(),
    )
}

// ---------------------------------------------------------------------------
// Span selection
// ---------------------------------------------------------------------------

/// Returns whether a span carries the given `sagashop.event` value.
#[must_use]
pub fn is_event(span: &CapturedSpan, event: &str) -> bool {
    span.sagashop_text("sagashop.event") == Some(event)
}

/// Returns whether a span is the given `sagashop.event` for the given order.
#[must_use]
pub fn is_event_for_order(span: &CapturedSpan, event: &str, order: &str) -> bool {
    is_event(span, event) && span.sagashop_text("sagashop.order.id") == Some(order)
}

/// Returns the distinct order identifiers a capture mentions, in stable order.
#[must_use]
pub fn order_ids(capture: &Capture) -> Vec<String> {
    capture
        .spans
        .iter()
        .filter_map(|span| span.sagashop_text("sagashop.order.id").map(str::to_owned))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Returns the single order identifier that a capture ships.
///
/// # Panics
///
/// Panics unless exactly one order ships, which the fixture-truth notes in
/// `testdata/sagashop/README.md` record for each vendored capture.
#[must_use]
pub fn only_shipped_order(capture: &Capture) -> String {
    let shipped: BTreeSet<String> = capture
        .spans
        .iter()
        .filter(|span| is_event(span, ORDER_SHIP))
        .filter_map(|span| span.sagashop_text("sagashop.order.id").map(str::to_owned))
        .collect();
    assert_eq!(shipped.len(), 1, "expected exactly one shipped order");
    shipped.into_iter().next().expect("one shipped order")
}

fn position_of(capture: &Capture, predicate: &dyn Fn(&CapturedSpan) -> bool) -> usize {
    let matches: Vec<usize> = capture
        .spans
        .iter()
        .enumerate()
        .filter(|(_, span)| predicate(span))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "mutation helpers require exactly one matching span, found {}",
        matches.len()
    );
    matches[0]
}

// ---------------------------------------------------------------------------
// Mutation helpers
// ---------------------------------------------------------------------------

/// Removes one attribute from the single span the predicate selects.
///
/// Sequences are untouched, so no gap is produced.
pub fn remove_span_attribute(
    capture: &mut Capture,
    predicate: &dyn Fn(&CapturedSpan) -> bool,
    key: &str,
) {
    let position = position_of(capture, predicate);
    assert!(
        capture.spans[position].attributes.remove(key).is_some(),
        "span does not carry {key}"
    );
}

/// Sets one string attribute on the single span the predicate selects.
///
/// Sequences are untouched, so no gap is produced.
pub fn set_span_attribute(
    capture: &mut Capture,
    predicate: &dyn Fn(&CapturedSpan) -> bool,
    key: &str,
    value: &str,
) {
    let position = position_of(capture, predicate);
    capture.spans[position]
        .attributes
        .insert(key.to_owned(), CaptureAttribute::String(value.to_owned()));
}

/// Deletes the selected span **and renumbers** its service to stay contiguous.
///
/// Contract 1 of the module documentation: renumbering keeps the evidence
/// gate's G2 disjunct out of the way, so whatever the test is actually about is
/// the only thing that can downgrade the Verdict.
pub fn delete_span(capture: &mut Capture, predicate: &dyn Fn(&CapturedSpan) -> bool) {
    let position = position_of(capture, predicate);
    let service = capture.spans[position].service.clone();
    capture.spans.remove(position);
    renumber_source_sequence(capture, &service);
}

/// Deletes every span of one service, making that source **unobserved**.
///
/// Unlike [`delete_span`], which removes a single span and renumbers, this
/// removes an entire service's evidence so the source emits nothing at all.
/// Used by the missing-required-source tests (Test 7) to trigger the evidence
/// gate's G1 disjunct (required source absent) rather than G2 (sequence gap)
/// or the anchor-downgrade rule. No renumbering is needed: the deleted service
/// has no remaining spans, and every other service keeps its contiguous
/// `sourceSequence` set.
pub fn delete_all_spans_of_service(capture: &mut Capture, service: &str) {
    capture.spans.retain(|span| span.service != service);
}

/// Deletes the selected span and **leaves** the sequence discontinuous.
///
/// The single deliberate exception to contract 1. The selected span must not be
/// the highest `sourceSequence` of its service, or no gap is produced at all.
///
/// # Panics
///
/// Panics when the selected span is the last of its service by sequence.
pub fn delete_span_leaving_sequence_gap(
    capture: &mut Capture,
    predicate: &dyn Fn(&CapturedSpan) -> bool,
) {
    let position = position_of(capture, predicate);
    let service = capture.spans[position].service.clone();
    let sequence = capture.spans[position].source_sequence;
    let highest = capture
        .spans
        .iter()
        .filter(|span| span.service == service)
        .map(|span| span.source_sequence)
        .max()
        .expect("service has spans");
    assert!(
        sequence < highest,
        "deleting the last span of a service produces no gap"
    );
    capture.spans.remove(position);
}

/// Repoints the selected span's `logicalParentSpanId` at an absent identifier.
///
/// Contract 2 of the module documentation: this produces a dangling causal
/// predecessor with zero sequence disturbance, isolating the gate's G4 disjunct.
pub fn retarget_logical_parent(
    capture: &mut Capture,
    predicate: &dyn Fn(&CapturedSpan) -> bool,
    absent_parent: &str,
) {
    assert_eq!(
        absent_parent.len(),
        16,
        "a logical span id is 16 hex digits"
    );
    assert!(
        !capture
            .spans
            .iter()
            .any(|span| span.logical_span_id == absent_parent),
        "the retargeted parent must name no span in the capture"
    );
    let position = position_of(capture, predicate);
    capture.spans[position].logical_parent_span_id = Some(absent_parent.to_owned());
}

/// Copies the selected span under a new `logicalSpanId` and a fresh sequence.
///
/// Contract 3 of the module documentation: the copy takes `max + 1` for its
/// service, so it occupies a free delivery slot rather than colliding with the
/// original. The service's completeness signal is derived from the maximum, so
/// it follows automatically.
pub fn duplicate_span(
    capture: &mut Capture,
    predicate: &dyn Fn(&CapturedSpan) -> bool,
    new_logical_span_id: &str,
) {
    assert_eq!(
        new_logical_span_id.len(),
        16,
        "a logical span id is 16 hex digits"
    );
    let position = position_of(capture, predicate);
    let mut copy = capture.spans[position].clone();
    let highest = capture
        .spans
        .iter()
        .filter(|span| span.service == copy.service)
        .map(|span| span.source_sequence)
        .max()
        .expect("service has spans");
    new_logical_span_id.clone_into(&mut copy.logical_span_id);
    new_logical_span_id.clone_into(&mut copy.span_id);
    copy.source_sequence = highest + 1;
    capture.spans.push(copy);
}

/// Gives the selected span the same `logicalSpanId` as another span.
///
/// The two spans then produce Events with the same identity in different
/// delivery slots, which is an `IdentityConflict` rather than a duplicate.
pub fn reuse_logical_span_id(
    capture: &mut Capture,
    predicate: &dyn Fn(&CapturedSpan) -> bool,
    donor: &dyn Fn(&CapturedSpan) -> bool,
) {
    let donor_id = capture.spans[position_of(capture, donor)]
        .logical_span_id
        .clone();
    let position = position_of(capture, predicate);
    capture.spans[position]
        .logical_span_id
        .clone_from(&donor_id);
    capture.spans[position].span_id = donor_id;
}

/// Makes two order spans mutually name each other's state as their previous one.
///
/// Edge class 4 then emits `a.completed -> b.requested` and
/// `b.completed -> a.requested`, which with edge class 1 closes a cycle that is
/// visible only transitively. `shared/monitor.ts` guards the same shape with
/// its own `cycle in unique order-transition graph` check.
pub fn make_mutual_transition_cycle(
    capture: &mut Capture,
    left: &dyn Fn(&CapturedSpan) -> bool,
    right: &dyn Fn(&CapturedSpan) -> bool,
) {
    let left_state = capture.spans[position_of(capture, left)]
        .sagashop_text("sagashop.order.state")
        .expect("left span records a state")
        .to_owned();
    let right_state = capture.spans[position_of(capture, right)]
        .sagashop_text("sagashop.order.state")
        .expect("right span records a state")
        .to_owned();
    set_span_attribute(capture, left, "sagashop.order.prev_state", &right_state);
    set_span_attribute(capture, right, "sagashop.order.prev_state", &left_state);
}

fn renumber_source_sequence(capture: &mut Capture, service: &str) {
    let mut ordered: Vec<(u64, usize)> = capture
        .spans
        .iter()
        .enumerate()
        .filter(|(_, span)| span.service == service)
        .map(|(index, span)| (span.source_sequence, index))
        .collect();
    ordered.sort_unstable();
    for (renumbered, (_, index)) in ordered.into_iter().enumerate() {
        capture.spans[index].source_sequence =
            u64::try_from(renumbered + 1).expect("sequence fits in u64");
    }
}

// ---------------------------------------------------------------------------
// Event-level helpers
// ---------------------------------------------------------------------------

/// Returns the adapted Event with the given identity.
///
/// # Panics
///
/// Panics when no Event carries the identity.
#[must_use]
pub fn event_named(evidence: &ObservedEvidence, id: &str) -> Event {
    evidence
        .events()
        .iter()
        .find(|event| event.id().as_str() == id)
        .unwrap_or_else(|| panic!("evidence carries an Event named {id}"))
        .clone()
}

/// Returns the single Event of the given kind for the given order.
///
/// # Panics
///
/// Panics unless exactly one Event matches.
#[must_use]
pub fn only_event(evidence: &ObservedEvidence, kind: &str, order: &str) -> Event {
    let matches: Vec<&Event> = evidence
        .events()
        .iter()
        .filter(|event| {
            event.producer().kind().as_str() == kind
                && event
                    .producer()
                    .attributes()
                    .get(&AttributeKey::new("sagashop.order.id").expect("fixture"))
                    == Some(&AttributeValue::String(order.to_owned()))
        })
        .collect();
    assert_eq!(matches.len(), 1, "expected exactly one {kind} for {order}");
    matches[0].clone()
}

/// Rebuilds an Event with one attribute replaced, keeping its identity and slot.
///
/// Same identity and slot with different producer evidence is an
/// `EvidenceConflict`. This is built at the Event level deliberately: the
/// Adapter refuses to emit two Events for one delivery slot, so a Core-level
/// delivery conflict cannot be reached through a capture mutation.
///
/// # Panics
///
/// Panics when the rebuilt producer evidence fails model validation.
#[must_use]
pub fn with_altered_attribute(event: &Event, key: &str, value: &str) -> Event {
    let producer = event.producer();
    let mut attributes = producer.attributes();
    attributes.insert(
        AttributeKey::new(key).expect("fixture attribute key"),
        AttributeValue::String(value.to_owned()),
    );
    rebuild(
        event,
        event.id().as_str(),
        event.delivery().sequence().value(),
        attributes,
    )
}

/// Rebuilds an Event under a different identity, keeping its delivery slot.
///
/// Different identity claiming the same slot is a `SlotConflict`.
///
/// # Panics
///
/// Panics when the rebuilt producer evidence fails model validation.
#[must_use]
pub fn with_altered_identity(event: &Event, id: &str) -> Event {
    rebuild(
        event,
        id,
        event.delivery().sequence().value(),
        event.producer().attributes(),
    )
}

/// Rebuilds an Event in a different delivery slot, keeping its identity.
///
/// The same identity claiming a different slot is an `IdentityConflict`.
///
/// # Panics
///
/// Panics when the rebuilt producer evidence fails model validation.
#[must_use]
pub fn with_altered_slot(event: &Event, sequence: u64) -> Event {
    rebuild(
        event,
        event.id().as_str(),
        sequence,
        event.producer().attributes(),
    )
}

fn rebuild(
    event: &Event,
    id: &str,
    sequence: u64,
    attributes: BTreeMap<AttributeKey, AttributeValue>,
) -> Event {
    let producer = event.producer();
    let slot = event.delivery();
    let rebuilt = sentinel_model::ProducerEvent::new(
        sentinel_model::EventId::new(id).expect("identity is not empty"),
        slot.epoch(),
        sentinel_model::SequenceNumber::new(sequence),
        producer.schema_version(),
        producer.filter_epoch(),
        producer.kind(),
        producer.occurrence_time(),
        attributes,
        producer.claimed_capabilities(),
        producer.sensitivity(),
        producer.causal_predecessors(),
    )
    .expect("rebuilt producer evidence is valid");
    Event::observe(
        rebuilt,
        event.assurance(),
        event.observation_time(),
        event.data_class(),
    )
}

/// One deterministic way of presenting the same Events to the Core.
pub type ArrivalOrder = Box<dyn Fn(&mut Vec<Event>)>;

/// Deterministic Fisher-Yates driven by a seeded linear congruential generator.
fn shuffle(events: &mut [Event], seed: u64) {
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    for index in (1..events.len()).rev() {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let pick = usize::try_from(state >> 33).expect("shift fits in usize") % (index + 1);
        events.swap(index, pick);
    }
}

/// Returns 36 deterministic arrival-order permutations (4 named + 32 shuffles).
///
/// `e2e_oracle.rs` keeps its own private copy of this generator and is
/// deliberately left untouched: it is the frozen baseline, and editing it to
/// share a helper would dissolve the guarantee that the catalogue-built
/// Specifications are being held to the *same* bar the hand-written ones already
/// passed. The duplication is the cheaper of the two costs.
#[must_use]
pub fn arrival_orders() -> Vec<ArrivalOrder> {
    let mut orders: Vec<ArrivalOrder> = vec![
        Box::new(|_events| {}),
        Box::new(|events| events.reverse()),
        Box::new(|events| events.sort_by_key(Event::id)),
        Box::new(|events| {
            events.sort_by_key(|event| {
                (
                    event.source(),
                    event.delivery().epoch().value(),
                    event.delivery().sequence().value(),
                )
            });
        }),
    ];
    for seed in 0..32_u64 {
        orders.push(Box::new(move |events| shuffle(events, seed)));
    }
    orders
}

/// Feeds the same evidence in a different arrival order.
#[must_use]
pub fn reordered(evidence: &ObservedEvidence, order: impl Fn(&mut Vec<Event>)) -> ObservedEvidence {
    let mut events = evidence.events().to_vec();
    order(&mut events);
    evidence.with_events(events)
}

// ---------------------------------------------------------------------------
// Issue #12: stepping, cold guards and evidence mutation
// ---------------------------------------------------------------------------

/// A declared cold-path guard the testbed Specification can carry.
pub const REACHABILITY_GUARD: &str = "sagashop.p4.charge-reachability";
/// A second declared guard, so batching has more than one query per Binding.
pub const SETTLEMENT_GUARD: &str = "sagashop.p4.charge-settlement";

/// Returns the two test-only cold guards.
///
/// These are **fixture** guards, exactly as `ship_requires_prior_charge` is a
/// fixture Specification. `FactQuery` is uninterpreted by the Core, so a guard
/// id is all #12 needs and all #12 is entitled to invent: #18 owns what a guard
/// evaluates.
#[must_use]
pub fn cold_guards() -> BTreeSet<ColdGuardId> {
    BTreeSet::from([
        ColdGuardId::new(REACHABILITY_GUARD, "v1"),
        ColdGuardId::new(SETTLEMENT_GUARD, "v1"),
    ])
}

/// The testbed Specification with two declared cold-path guards.
#[must_use]
pub fn ship_requires_prior_charge_with_cold_guards() -> PrecedenceSpecification {
    ship_requires_prior_charge().with_cold_guards(cold_guards())
}

/// Returns the driver's input sequence: every Event, then every source.
///
/// This is exactly what `sentinel_core::evaluate` feeds, and the order is
/// deliberate: feeding evidence about completeness *after* the Events it
/// describes is the order the monotonicity contract is written for.
#[must_use]
pub fn inputs(evidence: &ObservedEvidence) -> Vec<Input> {
    event_inputs(evidence)
        .into_iter()
        .chain(source_inputs(evidence))
        .collect()
}

/// Returns one `Input::Event` per adapted Event, in Adapter emission order.
#[must_use]
pub fn event_inputs(evidence: &ObservedEvidence) -> Vec<Input> {
    evidence
        .events()
        .iter()
        .map(|event| Input::event(event.clone()))
        .collect()
}

/// Returns one `Input::Source` per source that supplied evidence.
#[must_use]
pub fn source_inputs(evidence: &ObservedEvidence) -> Vec<Input> {
    evidence
        .sources()
        .values()
        .map(|record| Input::source(record.clone()))
        .collect()
}

/// Feeds a whole batch through the stepping API and seals it.
///
/// # Errors
///
/// Returns whatever the stepper returns.
pub fn drive(
    specification: &PrecedenceSpecification,
    inputs: Vec<Input>,
) -> Result<Vec<BindingVerdict>, CoreError> {
    let mut evaluator = Evaluator::new(specification.clone());
    for input in inputs {
        evaluator.step(input)?;
    }
    Ok(evaluator.seal()?.into_verdicts())
}

/// Encodes Verdicts for byte-identity comparison.
#[must_use]
pub fn encoded_verdicts(verdicts: &[BindingVerdict]) -> Vec<(String, Vec<u8>)> {
    verdicts
        .iter()
        .map(|binding| {
            (
                binding.binding().to_owned(),
                binding.verdict().encode_to_vec(),
            )
        })
        .collect()
}

/// Rebuilds one source's evidence with a different assurance tier.
#[must_use]
pub fn with_tier(evidence: &SourceEvidence, tier: SourceTier) -> SourceEvidence {
    SourceEvidence::new(
        evidence.source().clone(),
        tier,
        evidence.capabilities().clone(),
        evidence.completeness().clone(),
        evidence.gaps().to_vec(),
    )
}

/// Rebuilds one source's evidence with a different set of sequence gaps.
#[must_use]
pub fn with_gaps(
    evidence: &SourceEvidence,
    gaps: Vec<sentinel_model::SequenceGap>,
) -> SourceEvidence {
    SourceEvidence::new(
        evidence.source().clone(),
        evidence.tier(),
        evidence.capabilities().clone(),
        evidence.completeness().clone(),
        gaps,
    )
}

/// Rebuilds one source's evidence with different completeness bounds.
///
/// `sample` supplies the receiver-owned `SourceAssurance`, which
/// `SourceCompletenessSignal` exposes no accessor for; any Event from the same
/// source carries the identical one.
///
/// # Panics
///
/// Panics when the rebuilt signal fails model validation, which means the test
/// asked for bounds the model itself forbids.
#[must_use]
pub fn with_bounds(
    evidence: &SourceEvidence,
    sample: &Event,
    epoch: u64,
    last_emitted: Option<u64>,
    complete_through: Option<u64>,
) -> SourceEvidence {
    let signal = evidence.completeness();
    let producer = sentinel_model::ProducerCompletenessSignal::new(
        sentinel_model::SequenceEpoch::new(epoch),
        last_emitted.map(sentinel_model::SequenceNumber::new),
        complete_through.map(sentinel_model::SequenceNumber::new),
        signal.producer_reported_at(),
        signal.filter_epoch(),
        signal.activity(),
    )
    .expect("rebuilt completeness bounds are valid");
    SourceEvidence::new(
        evidence.source().clone(),
        evidence.tier(),
        evidence.capabilities().clone(),
        sentinel_model::SourceCompletenessSignal::observe(
            producer,
            sample.assurance(),
            signal.observed_at(),
        ),
        evidence.gaps().to_vec(),
    )
}

/// Builds the activation barrier the adapted evidence already satisfies, at a chosen
/// target Filter epoch.
///
/// Each required source's Watermark is taken from the evidence it actually declared —
/// same sequence epoch, same completeness boundary, same Sentinel observation time — so
/// the **only** variable is `target_filter_epoch`. Passing the epoch the sources
/// acknowledged yields a barrier that clears; passing a higher one yields a barrier that
/// is pending for exactly the reason a real rollout is pending, namely sources still
/// shipping under a stale Event Filter.
///
/// # Panics
///
/// Panics when a required source declared no completeness boundary, which would mean the
/// vendored capture no longer supports the fixture.
#[must_use]
pub fn activation_barrier(
    evidence: &ObservedEvidence,
    target_filter_epoch: u64,
) -> ActivationBarrier {
    let watermarks = [ORDER_SERVICE, PAYMENT_SERVICE]
        .iter()
        .map(|service| {
            let identity = source(service);
            let signal = evidence
                .source(&identity)
                .unwrap_or_else(|| panic!("{service} supplied evidence"))
                .completeness();
            let watermark = ActivationWatermark::new(
                signal.epoch(),
                signal
                    .complete_through()
                    .expect("the fixture declares a completeness boundary"),
                signal.observed_at(),
            );
            (identity, watermark)
        })
        .collect();
    ActivationBarrier::new(FilterEpoch::new(target_filter_epoch), watermarks)
        .expect("the fixture names two required sources")
}

/// Rebuilds one source's evidence with a different acknowledged Filter epoch.
///
/// The SagaShop Adapter stamps a constant epoch on every signal, so nothing in the
/// vendored fixtures varies this field. Without a helper the Verdict's `filter_epochs`
/// would be pinned only against the value a constant `0` happens to agree with.
///
/// `sample` supplies the receiver-owned `SourceAssurance`, exactly as
/// [`with_bounds`] does; any Event from the same source carries the identical one.
///
/// # Panics
///
/// Panics when the rebuilt signal fails model validation.
#[must_use]
pub fn with_filter_epoch(
    evidence: &SourceEvidence,
    sample: &Event,
    filter_epoch: u64,
) -> SourceEvidence {
    let signal = evidence.completeness();
    let producer = sentinel_model::ProducerCompletenessSignal::new(
        signal.epoch(),
        signal.last_emitted(),
        signal.complete_through(),
        signal.producer_reported_at(),
        FilterEpoch::new(filter_epoch),
        signal.activity(),
    )
    .expect("rebuilt Filter epoch is valid");
    SourceEvidence::new(
        evidence.source().clone(),
        evidence.tier(),
        evidence.capabilities().clone(),
        sentinel_model::SourceCompletenessSignal::observe(
            producer,
            sample.assurance(),
            signal.observed_at(),
        ),
        evidence.gaps().to_vec(),
    )
}

/// Returns the Watermark point each named source declared.
///
/// This is the hot settlement point, which is also the floor an answer has to
/// reach, so a response built from it settles exactly.
///
/// # Panics
///
/// Panics when a named source supplied no evidence.
#[must_use]
pub fn declared_watermarks(
    evidence: &ObservedEvidence,
    services: &[&str],
) -> BTreeMap<SourceIdentity, (SequenceEpoch, SequenceNumber)> {
    services
        .iter()
        .map(|service| {
            let identity = source(service);
            let signal = evidence
                .source(&identity)
                .unwrap_or_else(|| panic!("{service} supplied evidence"))
                .completeness();
            let point = (
                signal.epoch(),
                signal
                    .complete_through()
                    .expect("the fixture declares a completeness boundary"),
            );
            (identity, point)
        })
        .collect()
}

/// Returns the required-source Watermark points, which every answer must cover.
#[must_use]
pub fn required_watermarks(
    evidence: &ObservedEvidence,
) -> BTreeMap<SourceIdentity, (SequenceEpoch, SequenceNumber)> {
    declared_watermarks(evidence, &[ORDER_SERVICE, PAYMENT_SERVICE])
}

/// Answers every query in a request at the given Watermark.
#[must_use]
pub fn answer_all(
    request: &FactRequest,
    settled_at: &BTreeMap<SourceIdentity, (SequenceEpoch, SequenceNumber)>,
) -> Vec<FactResponse> {
    answer_all_as(request, &GuardConclusion::Holds, settled_at)
}

/// Answers every query in a request with one conclusion at the given Watermark.
#[must_use]
pub fn answer_all_as(
    request: &FactRequest,
    conclusion: &GuardConclusion,
    settled_at: &BTreeMap<SourceIdentity, (SequenceEpoch, SequenceNumber)>,
) -> Vec<FactResponse> {
    request
        .queries()
        .iter()
        .map(|query| FactResponse::new(query.clone(), conclusion.clone(), settled_at.clone()))
        .collect()
}

/// Adds two extra ship triggers under fresh order identifiers.
///
/// Three Bindings times two guards is what makes "one round trip, six queries"
/// a real batching pin rather than a restatement of "one query".
///
/// # Panics
///
/// Panics when the capture does not ship exactly one order.
pub fn add_shipped_orders(capture: &mut Capture, orders: &[(&str, &str)]) {
    let original = capture
        .spans
        .iter()
        .find(|span| is_event(span, ORDER_SHIP))
        .expect("the capture ships an order")
        .logical_span_id
        .clone();
    for (span_id, order) in orders {
        duplicate_span(capture, &|span| span.logical_span_id == original, span_id);
        set_span_attribute(
            capture,
            &|span| span.logical_span_id == *span_id,
            "sagashop.order.id",
            order,
        );
        // The copy is a causally isolated trigger, exactly as the mixed-outcome
        // fixture arranges: without this the strict transition predicate sees
        // two candidates leaving the same state and emits nothing anyway.
        remove_span_attribute(
            capture,
            &|span| span.logical_span_id == *span_id,
            "sagashop.order.prev_state",
        );
    }
}

// ---------------------------------------------------------------------------
// Issue #13: E2E oracle checks (P1, P2, P3, P5, P7)
// ---------------------------------------------------------------------------

/// The outcome of an oracle check: the property holds or is violated.
///
/// Oracle checks return `Pass` for vacuous cases — when the target events are
/// entirely absent, the property holds vacuously. ADR-0017's "never a silent
/// pass" is about hiding violations, not re-reporting vacuous truth as unknown.
/// There is no `Unevaluated` variant: absence of target events is `Pass`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OracleOutcome {
    /// The property holds (including vacuously, when no target events exist).
    Pass,
    /// The property is violated.
    Fail,
}

/// Evaluates one oracle property against adapted evidence.
///
/// Supports P1, P2, P3, P5, and P7. The precedence properties P4, P6, P8 are
/// evaluated by the Core's [`evaluate`] function, not by this oracle.
///
/// # Panics
///
/// Panics for an unknown property identifier.
#[must_use]
pub fn oracle_check(property: &str, evidence: &ObservedEvidence) -> OracleOutcome {
    match property {
        "P1" => no_ship_from_cancelled(evidence),
        "P2" => at_most_one_successful_charge(evidence),
        "P3" => cancel_is_terminal(evidence),
        "P5" => inventory_monotonic_decrement(evidence),
        "P7" => reserve_requires_prior_create(evidence),
        _ => panic!("unknown oracle property: {property}"),
    }
}

/// Returns whether an Event's string attribute equals the given value.
fn attr_is(event: &Event, key: &str, value: &str) -> bool {
    event
        .producer()
        .attributes()
        .get(&AttributeKey::new(key).expect("fixture"))
        == Some(&AttributeValue::String(value.to_owned()))
}

/// P1: no ship may depart from a cancelled order.
///
/// Fails if any `order.ship` Event carries `sagashop.order.prev_state = CANCELLED`.
/// Vacuous when no ship Events exist.
#[must_use]
fn no_ship_from_cancelled(evidence: &ObservedEvidence) -> OracleOutcome {
    let violated = evidence.events().iter().any(|event| {
        event.producer().kind().as_str() == "sagashop.order.ship.requested"
            && attr_is(event, "sagashop.order.prev_state", "CANCELLED")
    });
    if violated {
        OracleOutcome::Fail
    } else {
        OracleOutcome::Pass
    }
}

/// P2: at most one successful charge per order.
///
/// Fails if any order has more than one `payment.charge.completed` Event with
/// `sagashop.payment.status = success`. Vacuous when no successful charges exist.
#[must_use]
fn at_most_one_successful_charge(evidence: &ObservedEvidence) -> OracleOutcome {
    let mut charges_per_order: BTreeMap<String, usize> = BTreeMap::new();
    for event in evidence.events() {
        if event.producer().kind().as_str() == "sagashop.payment.charge.completed"
            && attr_is(event, "sagashop.payment.status", "success")
        {
            if let Some(AttributeValue::String(order)) = event
                .producer()
                .attributes()
                .get(&AttributeKey::new("sagashop.order.id").expect("fixture"))
            {
                *charges_per_order.entry(order.clone()).or_default() += 1;
            }
        }
    }
    if charges_per_order.values().any(|&count| count > 1) {
        OracleOutcome::Fail
    } else {
        OracleOutcome::Pass
    }
}

/// P3: once cancelled, an order must be refunded (cancel is terminal).
///
/// Fails if any Event carries `prev_state = CANCELLED` with a `state` that is
/// not `REFUNDED`. The `prev_state` already encodes ordering (S3 fix), so no
/// separate causal check is needed. Vacuous when no Event carries
/// `prev_state = CANCELLED`.
#[must_use]
fn cancel_is_terminal(evidence: &ObservedEvidence) -> OracleOutcome {
    let violated = evidence.events().iter().any(|event| {
        attr_is(event, "sagashop.order.prev_state", "CANCELLED")
            && !attr_is(event, "sagashop.order.state", "REFUNDED")
    });
    if violated {
        OracleOutcome::Fail
    } else {
        OracleOutcome::Pass
    }
}

/// P5: inventory `stock_after` must strictly decrease per item.
///
/// Fails iff any two same-item reserves have non-decreasing `stock_after` when
/// one is causally before the other, OR iff all same-item reserves are pairwise
/// concurrent and any two share the same `stock_after` (no decrement at all).
/// The second condition handles the H3 hazard: 15 concurrent reserves all
/// reporting `stock_after = 9`. Vacuous when no reserves exist.
#[must_use]
fn inventory_monotonic_decrement(evidence: &ObservedEvidence) -> OracleOutcome {
    let by_id: BTreeMap<String, &Event> = evidence
        .events()
        .iter()
        .map(|e| (e.id().as_str().to_owned(), e))
        .collect();

    let mut reserves_by_item: BTreeMap<String, Vec<&Event>> = BTreeMap::new();
    for event in evidence.events() {
        if event.producer().kind().as_str() == "sagashop.inventory.reserve.completed"
            && event
                .producer()
                .attributes()
                .get(&AttributeKey::new("sagashop.inventory.reserved").expect("fixture"))
                == Some(&AttributeValue::Bool(true))
        {
            if let Some(AttributeValue::String(item)) = event
                .producer()
                .attributes()
                .get(&AttributeKey::new("sagashop.inventory.item").expect("fixture"))
            {
                reserves_by_item
                    .entry(item.clone())
                    .or_default()
                    .push(event);
            }
        }
    }

    for reserves in reserves_by_item.values() {
        if reserves.len() <= 1 {
            continue;
        }

        let mut any_causal_pair = false;
        for i in 0..reserves.len() {
            for j in 0..reserves.len() {
                if i == j {
                    continue;
                }
                if causal_reaches(&by_id, reserves[i].id().as_str(), reserves[j].id().as_str()) {
                    any_causal_pair = true;
                    if stock_after(reserves[i]) <= stock_after(reserves[j]) {
                        return OracleOutcome::Fail;
                    }
                }
            }
        }

        if !any_causal_pair {
            let stocks: Vec<i64> = reserves.iter().map(|e| stock_after(e)).collect();
            for i in 0..stocks.len() {
                for j in (i + 1)..stocks.len() {
                    if stocks[i] == stocks[j] {
                        return OracleOutcome::Fail;
                    }
                }
            }
        }
    }
    OracleOutcome::Pass
}

/// P7: every inventory reserve must have a prior order create for the same order.
///
/// For each `inventory.reserve.requested` Event, verifies an
/// `order.create.requested` Event exists for the same `sagashop.order.id` with
/// causal Before ordering. This is an oracle check rather than a
/// PrecedenceSpecification because the requirement (`order.create.requested`)
/// comes from a Contributing-tier source (`sagashop-order`), so the Core's
/// anchor rule could never produce an authoritative verdict for it. Vacuous
/// when no reserves exist.
#[must_use]
fn reserve_requires_prior_create(evidence: &ObservedEvidence) -> OracleOutcome {
    let by_id: BTreeMap<String, &Event> = evidence
        .events()
        .iter()
        .map(|e| (e.id().as_str().to_owned(), e))
        .collect();

    for event in evidence.events() {
        if event.producer().kind().as_str() != "sagashop.inventory.reserve.requested" {
            continue;
        }
        let Some(AttributeValue::String(order)) = event
            .producer()
            .attributes()
            .get(&AttributeKey::new("sagashop.order.id").expect("fixture"))
            .cloned()
        else {
            continue;
        };

        let has_prior_create = evidence.events().iter().any(|create| {
            create.producer().kind().as_str() == "sagashop.order.create.requested"
                && create
                    .producer()
                    .attributes()
                    .get(&AttributeKey::new("sagashop.order.id").expect("fixture"))
                    .is_some_and(|v| *v == AttributeValue::String(order.clone()))
                && causal_reaches(&by_id, create.id().as_str(), event.id().as_str())
        });

        if !has_prior_create {
            return OracleOutcome::Fail;
        }
    }
    OracleOutcome::Pass
}

/// Returns the `sagashop.inventory.stock_after` integer value of an Event.
fn stock_after(event: &Event) -> i64 {
    match event
        .producer()
        .attributes()
        .get(&AttributeKey::new("sagashop.inventory.stock_after").expect("fixture"))
    {
        Some(AttributeValue::Integer(v)) => *v,
        _ => i64::MAX,
    }
}

/// Returns whether `earlier` is a transitive causal predecessor of `later`.
///
/// Walks backward from `later` through `causal_predecessors` looking for
/// `earlier`. Events whose predecessor IDs name no Event in `by_id` (dangling
/// edges) simply terminate that walk branch.
fn causal_reaches(by_id: &BTreeMap<String, &Event>, earlier: &str, later: &str) -> bool {
    let mut visited = BTreeSet::new();
    let mut stack = vec![later.to_owned()];
    while let Some(current) = stack.pop() {
        if current == earlier {
            return true;
        }
        if !visited.insert(current.clone()) {
            continue;
        }
        if let Some(event) = by_id.get(&current) {
            for pred in event.causal_predecessors() {
                stack.push(pred.to_string());
            }
        }
    }
    false
}

/// Returns the distinct order identifiers that carry a given `sagashop.event`.
#[must_use]
pub fn orders_with_event(capture: &Capture, event: &str) -> Vec<String> {
    let ids: BTreeSet<String> = capture
        .spans
        .iter()
        .filter(|span| is_event(span, event))
        .filter_map(|span| span.sagashop_text("sagashop.order.id").map(str::to_owned))
        .collect();
    ids.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Issue #22: the pending-world fixture and the enforcing Filter
// ---------------------------------------------------------------------------

/// The Filter epoch the vendored captures acknowledge and every decision
/// fixture runs at.
pub const DECISION_FILTER_EPOCH: u64 = sentinel_ingest::sagashop::FILTER_EPOCH;
/// The request identifier every decision fixture echoes back.
pub const DECISION_REQUEST_ID: &str = "decision-fixture-request";
/// A transport budget comfortably above every fixture's declared budget.
pub const TRANSPORT_BUDGET_NANOSECONDS: u64 = 1_000_000_000;
/// The declared Enforcement Latency Budget the default fixture entry stamps.
pub const DECLARED_BUDGET_NANOSECONDS: u64 = 50_000_000;

/// Builds the pending world for one unmodified vendored capture.
#[must_use]
pub fn pending_world(scenario: &str) -> (ObservedEvidence, Event) {
    pending_world_from(&load_capture(scenario))
}

/// Builds the pending world from an already-mutated capture.
///
/// The pending-world invariant, stated once because every gate-open fixture
/// depends on it: the `happy` capture contains the action's causal
/// *descendants* — the order service's client span for the shipping call
/// declares the ship span as its logical parent, and the shipping service's
/// `shipping.schedule` span descends from that — so a fixture deleting only
/// the ship span leaves a dangling rule-2 edge, G4 closes the evidence gate,
/// and every intended row collapses into row 9. This builder therefore holds
/// **exactly the action's causal past**: the ship span, every span that
/// transitively names it as a logical parent, and the spans descending from
/// those are removed, and the affected services' sequences are renumbered to
/// stay contiguous (the module's contract 1). The `order.deliver` span stays:
/// it is not a logical child of the ship span, and its transition edge
/// self-heals — zero rule-4 candidates once the ship span is gone — rather
/// than dangling.
///
/// Returns the driven evidence plus the asked action: the ship Event adapted
/// from the capture **before** removal, so its identity, attributes and
/// declared causal predecessors are the real ones, and its declared past is
/// exactly what the driven evidence still holds.
///
/// # Panics
///
/// Panics when the capture does not ship exactly one order, or when either
/// adaptation fails, which would mean a fixture recipe broke the vendored
/// artifact's structure.
#[must_use]
pub fn pending_world_from(capture: &Capture) -> (ObservedEvidence, Event) {
    let full = adapt(capture);
    let order = only_shipped_order(capture);
    let pending = only_event(&full, "sagashop.order.ship.requested", &order);

    let ship_spans: BTreeSet<String> = capture
        .spans
        .iter()
        .filter(|span| is_event(span, ORDER_SHIP))
        .map(|span| span.logical_span_id.clone())
        .collect();
    assert_eq!(ship_spans.len(), 1, "the fixture ships exactly one action");
    let mut removed = ship_spans;
    loop {
        let descendants: BTreeSet<String> = capture
            .spans
            .iter()
            .filter(|span| {
                span.logical_parent_span_id
                    .as_ref()
                    .is_some_and(|parent| removed.contains(parent))
            })
            .map(|span| span.logical_span_id.clone())
            .collect();
        if descendants.is_subset(&removed) {
            break;
        }
        removed.extend(descendants);
    }

    let mut reduced = capture.clone();
    let affected: BTreeSet<String> = reduced
        .spans
        .iter()
        .filter(|span| removed.contains(&span.logical_span_id))
        .map(|span| span.service.clone())
        .collect();
    reduced
        .spans
        .retain(|span| !removed.contains(&span.logical_span_id));
    for service in affected {
        renumber_source_sequence(&mut reduced, &service);
    }
    (adapt(&reduced), pending)
}

/// Drives one evaluator over a world's evidence and hands back the monitor
/// state the seam borrows.
///
/// Deliberately not sealed: sealing is the batch driver's statement that no
/// further input will ever arrive, while a decision is asked mid-stream by
/// design.
#[must_use]
pub fn driven_evaluator(
    specification: &PrecedenceSpecification,
    evidence: &ObservedEvidence,
) -> Evaluator {
    let mut evaluator = Evaluator::new(specification.clone());
    for input in inputs(evidence) {
        evaluator
            .step(input)
            .expect("the fixture evidence steps cleanly");
    }
    evaluator
}

/// Builds the fixture's `DecideRequest` from the asked action's producer
/// evidence.
///
/// The producer evidence is the pending Event's own wire encoding, decoded
/// straight back out of the generated message — not a hand-rebuilt copy —
/// so the request carries the same identity, kind, attributes and causal
/// predecessors the fixture Event named. `ObservationTime`, Data Class and
/// `SourceAssurance` are not on `ProducerEvent` and cannot ride the request.
///
/// # Panics
///
/// Panics when the round trip or request validation fails, which would mean
/// the fixture assembled an action the Probe boundary refuses.
#[must_use]
pub fn decide_request(pending: &Event) -> DecideRequest {
    let raw_event = raw_model::Event::decode(pending.encode_to_vec().as_slice())
        .expect("the pending action round-trips its wire encoding");
    let producer = raw_event
        .producer_event
        .expect("a canonical Event carries producer evidence");
    DecideRequest::try_from(raw_probe::DecideRequest {
        request_id: DECISION_REQUEST_ID.to_owned(),
        idempotency_key: "decision-fixture-key".to_owned(),
        source_handle: ORDER_SERVICE.to_owned(),
        filter_epoch: Some(DECISION_FILTER_EPOCH),
        producer_event: Some(producer),
        remaining_transport_budget_nanoseconds: Some(TRANSPORT_BUDGET_NANOSECONDS),
    })
    .expect("the fixture request validates")
}

/// Hand-assembles one active-enforcement Filter entry for a Specification,
/// mirroring what Promotion (#166) will stamp: ask-and-block delivery,
/// Enforce, Active, every ADR-0023 gate attested, a nonzero declared
/// Enforcement Latency Budget, and one activation Watermark per required
/// source, taken from the completeness boundary that source actually
/// declared.
///
/// The `event_kinds` are the caller's, because the Filter is a sound
/// over-approximation: the aggregate fixtures deliberately widen an entry's
/// kind set past its Specification's own trigger kind, which is legal and is
/// the only way a charge action can be co-matched by two Specifications.
///
/// # Panics
///
/// Panics when a required source supplied no evidence, which would mean the
/// fixture world does not carry the entry's own required sources.
#[must_use]
pub fn enforcing_entry(
    evidence: &ObservedEvidence,
    specification: &PrecedenceSpecification,
    event_kinds: &[&str],
    fail_mode: raw_model::FailMode,
    latency_budget_nanoseconds: u64,
) -> raw_model::SpecificationFilter {
    let required: Vec<raw_model::SourceIdentity> = specification
        .required_source_identities()
        .iter()
        .map(sentinel_model::SourceIdentity::to_wire)
        .collect();
    let watermarks: Vec<raw_model::Watermark> = specification
        .required_source_identities()
        .iter()
        .map(|identity| {
            let signal = evidence
                .source(identity)
                .expect("a required source supplied evidence")
                .completeness();
            raw_model::Watermark {
                source: Some(identity.to_wire()),
                sequence_epoch: signal.epoch().value(),
                complete_through: signal
                    .complete_through()
                    .expect("the fixture declares a completeness boundary")
                    .value(),
                observed_at: Some(signal.observed_at().to_wire()),
            }
        })
        .collect();
    raw_model::SpecificationFilter {
        specification_id: specification.identity().id().to_owned(),
        specification_version: specification.identity().version().to_owned(),
        event_match: Some(raw_model::EventMatch {
            event_kinds: event_kinds.iter().map(|kind| (*kind).to_owned()).collect(),
            projected_attribute_keys: vec![SAGASHOP_BINDING_ATTRIBUTE.to_owned()],
            delivery_mode: raw_model::DeliveryMode::AskAndBlock as i32,
        }),
        fail_mode: fail_mode as i32,
        latency_budget_nanoseconds: Some(latency_budget_nanoseconds),
        evaluation_mode: raw_model::EvaluationMode::Enforce as i32,
        readiness: raw_model::Readiness::Active as i32,
        target_filter_epoch: Some(DECISION_FILTER_EPOCH),
        required_sources: required,
        activation_watermarks: watermarks,
        enforcement_gates: Some(raw_model::EnforcementGateEvidence {
            safety_property: true,
            ask_and_block: true,
            hot_path_only: true,
            anchor_tier: true,
            shadow_and_adversarial_validation_passed: true,
        }),
    }
}

/// Validates hand-assembled entries as the active Filter the Probe holds.
///
/// # Panics
///
/// Panics when the entries do not validate, which would mean the fixture
/// stopped mirroring what Promotion will stamp.
#[must_use]
pub fn enforcing_filter(entries: Vec<raw_model::SpecificationFilter>) -> EventFilter {
    EventFilter::try_from(raw_model::EventFilter {
        epoch: Some(DECISION_FILTER_EPOCH),
        specifications: entries,
        baseline_evidence: Some(raw_model::BaselineEvidenceFilter {
            source_health: true,
            completeness_signals: true,
        }),
    })
    .expect("the fixture mirrors what Promotion will stamp")
}

/// Hand-assembles one Warming/Shadow entry the matched set must ignore.
#[must_use]
pub fn shadow_entry(
    specification_id: &str,
    event_kinds: &[&str],
) -> raw_model::SpecificationFilter {
    raw_model::SpecificationFilter {
        specification_id: specification_id.to_owned(),
        specification_version: "v1".to_owned(),
        event_match: Some(raw_model::EventMatch {
            event_kinds: event_kinds.iter().map(|kind| (*kind).to_owned()).collect(),
            projected_attribute_keys: Vec::new(),
            delivery_mode: raw_model::DeliveryMode::ShipAsync as i32,
        }),
        fail_mode: raw_model::FailMode::Open as i32,
        latency_budget_nanoseconds: None,
        evaluation_mode: raw_model::EvaluationMode::Shadow as i32,
        readiness: raw_model::Readiness::Warming as i32,
        target_filter_epoch: Some(DECISION_FILTER_EPOCH),
        required_sources: Vec::new(),
        activation_watermarks: Vec::new(),
        enforcement_gates: None,
    }
}

/// Rebuilds an Event with one attribute removed, keeping its identity and slot.
///
/// The row-3 fixture's mutator: a governed action that carries no Binding
/// attribute. The rebuilt Event is asked, never delivered, so keeping the
/// derived identity and slot cannot collide with anything.
///
/// # Panics
///
/// Panics when the attribute was absent or the rebuilt producer evidence
/// fails model validation.
#[must_use]
pub fn with_removed_attribute(event: &Event, key: &str) -> Event {
    let producer = event.producer();
    let mut attributes = producer.attributes();
    assert!(
        attributes
            .remove(&AttributeKey::new(key).expect("fixture"))
            .is_some(),
        "the Event carries {key}"
    );
    rebuild(
        event,
        event.id().as_str(),
        event.delivery().sequence().value(),
        attributes,
    )
}

// ---------------------------------------------------------------------------
// Scope retirement (#110, ADR-0021's O-03)
// ---------------------------------------------------------------------------

/// Builds an observation time on the replay clock, from an integer literal.
///
/// **Every `ObservationTime` in the retirement suite comes through here.**
/// `.github/scripts/check-core-purity.sh` scans `core/src` and not
/// `core/tests`, so nothing mechanical stops a test reaching for `std::time` to
/// build one — and a retirement test that read a clock would be exercising the
/// wrong Core. One helper is one place to police that.
///
/// The Adapter stamps observation time as the capture position on the
/// `sentinel-replay` clock, so an integer here is directly comparable with an
/// adapted Event's own observation time.
///
/// # Panics
///
/// Never: the clock identifier is a non-empty constant.
#[must_use]
pub fn observation_time(nanoseconds: i128) -> ObservationTime {
    observation_time_on(sentinel_ingest::sagashop::SENTINEL_CLOCK_ID, nanoseconds)
}

/// Builds an observation time on a named Sentinel clock, from a literal.
///
/// # Panics
///
/// Panics when `clock` is blank.
#[must_use]
pub fn observation_time_on(clock: &str, nanoseconds: i128) -> ObservationTime {
    ObservationTime::new(
        SentinelClockId::new(clock).expect("the clock identifier is not empty"),
        nanoseconds,
    )
}

/// Builds a Scope horizon on the replay clock, from an integer literal.
#[must_use]
pub fn scope_horizon(nanoseconds: i128) -> ScopeHorizon {
    ScopeHorizon::new(observation_time(nanoseconds))
}

/// Builds a Scope horizon on a named Sentinel clock, from a literal.
#[must_use]
pub fn scope_horizon_on(clock: &str, nanoseconds: i128) -> ScopeHorizon {
    ScopeHorizon::new(observation_time_on(clock, nanoseconds))
}

/// Returns one retirement input for a Binding, at a horizon, for a cause.
#[must_use]
pub fn retire(binding: &str, nanoseconds: i128, cause: RetirementCause) -> Input {
    retire_at(binding, scope_horizon(nanoseconds), cause)
}

/// Returns one retirement input carrying an explicit horizon.
#[must_use]
pub fn retire_at(binding: &str, horizon: ScopeHorizon, cause: RetirementCause) -> Input {
    Input::retire(RetirementSignal::new(Binding::new(binding), horizon, cause))
}

/// The testbed P4 Specification, declaring a retirement policy.
///
/// AC1 is exercised through hand-built Specifications in addition to catalogued
/// CAT-3: `existence.before-scope-close` (#210) declares `Deadline` and makes
/// the mechanism reachable from catalogue data, but the retirement suite keeps
/// Precedence fixtures so clause (d0)/(d2)/(d3) pins stay independent of the
/// Existence decision table. CAT-1/CAT-2/CAT-4/CAT-5 remain Event-local
/// `NotRequired`.
#[must_use]
pub fn ship_requires_prior_charge_retiring(policy: RetirementPolicy) -> PrecedenceSpecification {
    ship_requires_prior_charge().with_retirement_policy(policy)
}

/// Returns one `Input::Source` per source **except** the named one.
///
/// The gate-closed fixture the retirement suite runs on: withholding one
/// required source's declaration closes the evidence gate through G1b while
/// leaving Evidence Coverage non-empty, which is exactly the state the retired
/// emission licence is defined over.
#[must_use]
pub fn source_inputs_except(evidence: &ObservedEvidence, withheld: &SourceIdentity) -> Vec<Input> {
    evidence
        .sources()
        .values()
        .filter(|record| record.source() != withheld)
        .map(|record| Input::source(record.clone()))
        .collect()
}

/// Returns the lowest and highest observation time across adapted Events.
///
/// Used to place a retirement horizon **strictly inside** the fixture's range,
/// so the in-scope Slice is a proper non-empty subset and admission
/// discriminates something rather than being asserted to be constant.
///
/// # Panics
///
/// Panics on an empty batch, which no vendored fixture is.
#[must_use]
pub fn observation_range(evidence: &ObservedEvidence) -> (i128, i128) {
    let mut times: Vec<i128> = evidence
        .events()
        .iter()
        .map(|event| event.observation_time().sentinel_nanoseconds())
        .collect();
    times.sort_unstable();
    (
        *times.first().expect("the batch is not empty"),
        *times.last().expect("the batch is not empty"),
    )
}
