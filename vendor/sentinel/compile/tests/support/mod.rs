//! Instantiation builders shared by `compile/tests/instantiation.rs` and
//! `compile/tests/subsumption.rs`.
//!
//! Integration test binaries do not share code unless a module is included into
//! each, so the conforming bindings for every catalogued Pattern live here
//! rather than being copied. `bindings_for` / `conforming` reach every family's
//! `*_arguments` / `*_evidence`, which is why the whole set moved together
//! (issue #177): copying only the CAT-1 half would not compile.
//!
//! The selectors are synthetic rather than SagaShop-shaped on purpose — see
//! `instantiation.rs`'s header for why.

#![allow(dead_code)] // Helpers are shared across test binaries; not all are used by each.
#![allow(clippy::doc_markdown)] // Product names and file paths read as prose here.

use std::collections::BTreeMap;

use sentinel_compile::catalogue::{
    instantiate, EvidenceBinding, ParameterValue, PatternId, PatternInstance, ProducerContract,
    SelectorValue,
};
use sentinel_compile::FilterIdentity;
use sentinel_model::{
    AttributeKey, AttributeValue, EventKind, MonitoredApplicationId, SchemaVersion,
    SourceCapabilities, SourceCapability, SourceId, SourceIdentity, SourceTier,
};

// ---------------------------------------------------------------------------
// Shared primitives
// ---------------------------------------------------------------------------

/// The Binding attribute every conforming instance is bound to.
pub const BINDING: &str = "example.request.id";

pub fn identity() -> FilterIdentity {
    FilterIdentity {
        specification_id: "example.action-needs-approval".to_owned(),
        specification_version: "v1".to_owned(),
    }
}

pub fn contract() -> ProducerContract {
    ProducerContract::new(SchemaVersion::new("example-contract-v1").expect("constant"))
}

pub fn source(application: &str) -> SourceIdentity {
    SourceIdentity::new(
        MonitoredApplicationId::new(application).expect("constant"),
        SourceId::new("probe").expect("constant"),
    )
}

pub fn attr_key(key: &str) -> AttributeKey {
    AttributeKey::new(key).expect("constant")
}

pub fn selector(kind: &str, attributes: &[(&str, &str)]) -> ParameterValue {
    let required = attributes
        .iter()
        .map(|(key, value)| (attr_key(key), AttributeValue::String((*value).to_owned())))
        .collect();
    ParameterValue::Selector(SelectorValue::new(
        EventKind::new(kind).expect("constant"),
        required,
    ))
}

/// Full capabilities: the deployment asserts everything CAT-1 asks for.
pub fn capable() -> SourceCapabilities {
    SourceCapabilities::new([
        SourceCapability::CausalEdges,
        SourceCapability::CompletenessSignals,
    ])
}

// ---------------------------------------------------------------------------
// Every catalogued Pattern
// ---------------------------------------------------------------------------

/// A conforming set of bindings for one catalogued Pattern.
///
/// The wildcard arm panics rather than skipping, and it has to be a wildcard:
/// [`PatternId`] is `#[non_exhaustive]`, so an integration test lives in another
/// crate and **cannot** write an exhaustive `match` over it. The compile-error
/// guarantee `PatternInstance::exercise` relies on is a within-crate guarantee
/// only; out here the equivalent is a loud panic naming the Pattern nobody
/// wrote a fixture for. Every catalogue loop in `instantiation.rs` and
/// `subsumption.rs` drives this function, so a Pattern added to the catalogue
/// fails all of them at once rather than being quietly left out of each.
pub fn bindings_for(
    pattern: PatternId,
) -> (
    BTreeMap<String, ParameterValue>,
    BTreeMap<String, EvidenceBinding>,
) {
    match pattern {
        PatternId::PrecedenceApproval => (arguments(), evidence(SourceTier::Anchor)),
        PatternId::ProvenanceTaint => (
            taint_arguments(),
            taint_evidence(SourceTier::Anchor, SourceTier::Anchor),
        ),
        PatternId::AbsenceAfterUntil => (
            absence_arguments(),
            absence_evidence(
                SourceTier::Contributing,
                SourceTier::Anchor,
                SourceTier::Anchor,
            ),
        ),
        PatternId::AggregationCountWithin => (
            aggregation_arguments(),
            aggregation_evidence(SourceTier::Anchor, SourceTier::Anchor),
        ),
        PatternId::ExistenceBeforeScopeClose => (
            existence_arguments(),
            existence_evidence(
                SourceTier::Contributing,
                SourceTier::Anchor,
                SourceTier::Contributing,
            ),
        ),
        other => panic!("{other} is catalogued with no conforming bindings in this suite"),
    }
}

/// Instantiates one catalogued Pattern from its conforming bindings.
pub fn conforming(pattern: PatternId) -> PatternInstance {
    let (arguments, evidence) = bindings_for(pattern);
    instantiate(pattern, identity(), contract(), arguments, evidence)
        .expect("a fully bound instantiation succeeds")
}

// ---------------------------------------------------------------------------
// CAT-1 precedence.approval
// ---------------------------------------------------------------------------

pub fn arguments() -> BTreeMap<String, ParameterValue> {
    BTreeMap::from([
        (
            "binding_attribute".to_owned(),
            ParameterValue::AttributeKey(attr_key(BINDING)),
        ),
        (
            "trigger".to_owned(),
            selector("example.action.executed", &[]),
        ),
        (
            "requirement".to_owned(),
            selector(
                "example.approval.granted",
                &[("example.approval.outcome", "allow")],
            ),
        ),
    ])
}

pub fn evidence(requirement_tier: SourceTier) -> BTreeMap<String, EvidenceBinding> {
    BTreeMap::from([
        (
            "trigger_source".to_owned(),
            EvidenceBinding::new(
                source("example-gateway"),
                SourceTier::Contributing,
                capable(),
            ),
        ),
        (
            "requirement_source".to_owned(),
            EvidenceBinding::new(source("example-authz"), requirement_tier, capable()),
        ),
    ])
}

// ---------------------------------------------------------------------------
// CAT-4 provenance.taint
// ---------------------------------------------------------------------------

/// The Event kind CAT-4's action selector selects.
pub const TAINT_ACTION_KIND: &str = "example.email.sent";

/// The Event kind CAT-4's untrusted-content selector selects.
pub const TAINT_MARKER_KIND: &str = "example.content.ingested";

pub fn taint_arguments() -> BTreeMap<String, ParameterValue> {
    BTreeMap::from([
        (
            "binding_attribute".to_owned(),
            ParameterValue::AttributeKey(attr_key(BINDING)),
        ),
        ("action".to_owned(), selector(TAINT_ACTION_KIND, &[])),
        (
            "untrusted_content".to_owned(),
            selector(TAINT_MARKER_KIND, &[("example.content.trust", "untrusted")]),
        ),
    ])
}

pub fn taint_evidence(
    action_tier: SourceTier,
    content_tier: SourceTier,
) -> BTreeMap<String, EvidenceBinding> {
    BTreeMap::from([
        (
            "action_source".to_owned(),
            EvidenceBinding::new(source("example-mail-gateway"), action_tier, capable()),
        ),
        (
            "content_source".to_owned(),
            EvidenceBinding::new(source("example-content-broker"), content_tier, capable()),
        ),
    ])
}

// ---------------------------------------------------------------------------
// CAT-2 absence.after-until
// ---------------------------------------------------------------------------

/// The Event kind CAT-2's after selector selects.
pub const ABSENCE_AFTER_KIND: &str = "example.approval.granted";

/// The Event kind CAT-2's until selector selects.
pub const ABSENCE_UNTIL_KIND: &str = "example.action.committed";

/// The Event kind CAT-2's veto selector selects.
pub const ABSENCE_VETO_KIND: &str = "example.approval.revoked";

pub fn absence_arguments() -> BTreeMap<String, ParameterValue> {
    BTreeMap::from([
        (
            "binding_attribute".to_owned(),
            ParameterValue::AttributeKey(attr_key(BINDING)),
        ),
        ("after".to_owned(), selector(ABSENCE_AFTER_KIND, &[])),
        ("until".to_owned(), selector(ABSENCE_UNTIL_KIND, &[])),
        ("veto".to_owned(), selector(ABSENCE_VETO_KIND, &[])),
    ])
}

pub fn absence_evidence(
    after_tier: SourceTier,
    until_tier: SourceTier,
    veto_tier: SourceTier,
) -> BTreeMap<String, EvidenceBinding> {
    BTreeMap::from([
        (
            "after_source".to_owned(),
            EvidenceBinding::new(source("example-approval-service"), after_tier, capable()),
        ),
        (
            "until_source".to_owned(),
            EvidenceBinding::new(source("example-action-gateway"), until_tier, capable()),
        ),
        (
            "veto_source".to_owned(),
            EvidenceBinding::new(source("example-revocation-bus"), veto_tier, capable()),
        ),
    ])
}

// ---------------------------------------------------------------------------
// CAT-5 aggregation.count-within
// ---------------------------------------------------------------------------

/// The Event kind CAT-5's trigger/assessment selector selects.
pub const AGGREGATION_TRIGGER_KIND: &str = "example.transfer.executed";

/// The Event kind CAT-5's member selector selects.
pub const AGGREGATION_MEMBER_KIND: &str = "example.transfer.attempted";

pub fn aggregation_arguments() -> BTreeMap<String, ParameterValue> {
    BTreeMap::from([
        (
            "binding_attribute".to_owned(),
            ParameterValue::AttributeKey(attr_key(BINDING)),
        ),
        (
            "trigger".to_owned(),
            selector(AGGREGATION_TRIGGER_KIND, &[]),
        ),
        ("member".to_owned(), selector(AGGREGATION_MEMBER_KIND, &[])),
        (
            "lookback".to_owned(),
            ParameterValue::SequenceLookbackBound(16),
        ),
        ("threshold".to_owned(), ParameterValue::CountThreshold(3)),
    ])
}

pub fn aggregation_evidence(
    trigger_tier: SourceTier,
    member_tier: SourceTier,
) -> BTreeMap<String, EvidenceBinding> {
    BTreeMap::from([
        (
            "trigger_source".to_owned(),
            EvidenceBinding::new(source("example-transfer-gateway"), trigger_tier, capable()),
        ),
        (
            "member_source".to_owned(),
            EvidenceBinding::new(source("example-transfer-ledger"), member_tier, capable()),
        ),
    ])
}

// ---------------------------------------------------------------------------
// CAT-3 existence.before-scope-close
// ---------------------------------------------------------------------------

/// The Event kind CAT-3's open selector selects.
pub const EXISTENCE_OPEN_KIND: &str = "example.saga.opened";

/// The Event kind CAT-3's required selector selects.
pub const EXISTENCE_REQUIRED_KIND: &str = "example.saga.compensated";

/// The Event kind CAT-3's close selector selects.
pub const EXISTENCE_CLOSE_KIND: &str = "example.saga.terminated";

pub fn existence_arguments() -> BTreeMap<String, ParameterValue> {
    BTreeMap::from([
        (
            "binding_attribute".to_owned(),
            ParameterValue::AttributeKey(attr_key(BINDING)),
        ),
        ("open".to_owned(), selector(EXISTENCE_OPEN_KIND, &[])),
        (
            "required".to_owned(),
            selector(EXISTENCE_REQUIRED_KIND, &[]),
        ),
        ("close".to_owned(), selector(EXISTENCE_CLOSE_KIND, &[])),
    ])
}

pub fn existence_evidence(
    open_tier: SourceTier,
    required_tier: SourceTier,
    close_tier: SourceTier,
) -> BTreeMap<String, EvidenceBinding> {
    BTreeMap::from([
        (
            "open_source".to_owned(),
            EvidenceBinding::new(source("example-saga-orchestrator"), open_tier, capable()),
        ),
        (
            "required_source".to_owned(),
            EvidenceBinding::new(source("example-money-system"), required_tier, capable()),
        ),
        (
            "close_source".to_owned(),
            EvidenceBinding::new(source("example-broker-journal"), close_tier, capable()),
        ),
    ])
}
