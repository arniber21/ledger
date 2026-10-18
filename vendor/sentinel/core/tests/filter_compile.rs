//! Issue #15 test 14: the Filter compiler against the testbed Specification.
//!
//! A test-only `FilterablePrecedence` newtype wraps the testbed
//! [`PrecedenceSpecification`] and implements [`FilterSource`] by delegating to
//! the spec's accessors. Keeping the impl in `core/tests` (not `core/src`)
//! avoids coupling production Core to `sentinel-compile` until #14 supplies real
//! Pattern impls (S3 fix from the adversarial review). The newtype owns the
//! wrapper type, so the foreign-trait impl is legal inside this test binary.
//!
//! #14a has since supplied that impl: `sentinel_compile::catalogue::PatternInstance`
//! implements `FilterSource` for real, and `core/tests/catalogue_grounding.rs`
//! compiles the P4/P6/P8 instances through it. This file stays because it tests
//! a different thing — the compiler's behaviour against a **hand-written**
//! Specification, with a deliberately tight budget chosen to force the
//! over-budget path. CAT-1's own budget is not tight, so the catalogue instances
//! cannot exercise that path.

#![allow(clippy::doc_markdown)] // Product names read as prose here.

mod support;

use std::collections::BTreeSet;

use sentinel_compile::{
    compile_filter, EventDependency, FilterIdentity, FilterSource, FilterWidthBudget,
};
use sentinel_core::specification::PrecedenceSpecification;
use sentinel_model::{
    wire::model::v1 as raw_model, AttributeKey, EventKind, FilterEpoch, SourceIdentity,
};

use support::{ship_requires_prior_charge, source};

/// Test-only adapter implementing `FilterSource` for the testbed
/// `PrecedenceSpecification`.
///
/// The declared width budget is deliberately tight (`1 + 1 = 2`) so the
/// over-budget path is exercised on the real fixture (the actual emitted width
/// is 4: two kinds + two keys).
struct FilterablePrecedence(PrecedenceSpecification);

impl FilterSource for FilterablePrecedence {
    fn filter_identity(&self) -> FilterIdentity {
        FilterIdentity {
            specification_id: self.0.identity().id().to_owned(),
            specification_version: self.0.identity().version().to_owned(),
        }
    }

    fn dependencies(&self) -> Vec<EventDependency> {
        // The trigger and requirement selectors are the two Events this
        // precedence spec could depend on. Their required_attributes are the
        // keys the Core reads when matching.
        vec![
            selector_dependency(self.0.trigger()),
            selector_dependency(self.0.requirement()),
        ]
    }

    fn binding_attribute_keys(&self) -> Vec<AttributeKey> {
        // The binding attribute is sliced from every admitted Event to name a
        // Binding. The trigger selector has empty required_attributes, so
        // without surfacing the binding attribute here the Core's binding_of()
        // would drop it -> silent Verdict inversion. Load-bearing.
        vec![self.0.binding_attribute().clone()]
    }

    fn required_sources(&self) -> Vec<SourceIdentity> {
        self.0.required_source_identities().into_iter().collect()
    }

    fn width_budget(&self) -> FilterWidthBudget {
        // Deliberately tight: actual is 2 kinds + 2 keys = 4 > declared 2.
        FilterWidthBudget {
            event_kinds: 1,
            attribute_keys: 1,
        }
    }
}

/// Projects one `EventSelector` into an `EventDependency` (kind + the keys it
/// slices on).
fn selector_dependency(selector: &sentinel_core::specification::EventSelector) -> EventDependency {
    EventDependency {
        kind: selector.kind().clone(),
        attribute_keys: selector.required_attributes().keys().cloned().collect(),
    }
}

#[test]
fn precedence_soundness_table_test() {
    let spec = FilterablePrecedence(ship_requires_prior_charge());
    let epoch = FilterEpoch::new(1);
    let compiled = compile_filter(&[&spec], epoch).expect("testbed compiles");

    assert_eq!(
        compiled.specifications().len(),
        1,
        "one SpecificationFilter"
    );
    let emitted = &compiled.specifications()[0];
    let event_match = emitted.event_match.as_ref().expect("event_match present");
    let kinds: BTreeSet<&str> = event_match.event_kinds.iter().map(String::as_str).collect();
    let keys: BTreeSet<&str> = event_match
        .projected_attribute_keys
        .iter()
        .map(String::as_str)
        .collect();

    // S1: both selector kinds are admitted (trigger + requirement).
    assert!(
        kinds.contains("sagashop.order.ship.requested"),
        "trigger kind not admitted: {kinds:?}"
    );
    assert!(
        kinds.contains("sagashop.payment.charge.completed"),
        "requirement kind not admitted: {kinds:?}"
    );

    // S2: the binding attribute (order.id) and the requirement's slicing
    // attribute (payment.status) are retained. order.id is load-bearing: the
    // trigger selector has empty required_attributes, so without
    // binding_attribute_keys the Core's binding_of() would drop it.
    assert!(
        keys.contains("sagashop.order.id"),
        "binding attribute not retained: {keys:?}"
    );
    assert!(
        keys.contains("sagashop.payment.status"),
        "requirement attribute not retained: {keys:?}"
    );

    // Width: actual 4 (2 kinds + 2 keys) > declared 2 -> over_budget, NOT
    // truncated (every kind/key still present above).
    let report = &compiled.widths[0];
    assert!(
        report.over_budget,
        "expected over_budget; report = {report:?}"
    );
    assert_eq!(report.actual_event_kinds, 2);
    assert_eq!(report.actual_attribute_keys, 2);
    assert_eq!(report.actual_width, 4);
    assert_eq!(report.declared_width, 2);

    // Defaults: Shadow / Warming / ShipAsync / FailOpen, epoch-stamped,
    // baseline-on (validated by construction).
    assert_eq!(
        emitted.evaluation_mode,
        raw_model::EvaluationMode::Shadow as i32
    );
    assert_eq!(emitted.readiness, raw_model::Readiness::Warming as i32);
    assert_eq!(
        event_match.delivery_mode,
        raw_model::DeliveryMode::ShipAsync as i32
    );
    assert_eq!(emitted.fail_mode, raw_model::FailMode::Open as i32);
    assert_eq!(emitted.target_filter_epoch, Some(epoch.value()));

    // Required sources preserved (order + payment services), via the same
    // `source()` helper the fixture uses.
    let expected_srcs: BTreeSet<(String, String)> =
        [source("sagashop-order"), source("sagashop-payment")]
            .iter()
            .map(|s| {
                (
                    s.monitored_application().to_string(),
                    s.source().to_string(),
                )
            })
            .collect();
    let emitted_srcs: BTreeSet<(String, String)> = emitted
        .required_sources
        .iter()
        .map(|s| (s.monitored_application_id.clone(), s.source_id.clone()))
        .collect();
    assert_eq!(
        emitted_srcs, expected_srcs,
        "required sources not preserved"
    );
}

/// Confirms `EventKind` is the macro product the adapter relies on (Clone,
/// `as_str`); a compile-time sanity anchor for the delegation above.
#[test]
fn event_kind_delegation_compiles() {
    let kind = EventKind::new("sagashop.order.ship.requested").expect("valid kind");
    assert_eq!(kind.as_str(), "sagashop.order.ship.requested");
}
