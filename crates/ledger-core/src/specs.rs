//! The shipped monitors: precedence, absence-after, response, and tainted-memo.
//!
//! Each one is a catalogue Pattern lowered into an [`Evaluator`]. Callers step
//! that evaluator and seal it. They do not call the batch `evaluate` path.

use std::collections::BTreeMap;

use sentinel_compile::catalogue::{
    instantiate, EvidenceBinding, ParameterValue, PatternId, ProducerContract, SelectorValue,
    ABSENCE_AFTER, ABSENCE_AFTER_SOURCE, ABSENCE_BINDING_ATTRIBUTE, ABSENCE_UNTIL,
    ABSENCE_UNTIL_SOURCE, ABSENCE_VETO, ABSENCE_VETO_SOURCE, EXISTENCE_BINDING_ATTRIBUTE,
    EXISTENCE_CLOSE, EXISTENCE_CLOSE_SOURCE, EXISTENCE_OPEN, EXISTENCE_OPEN_SOURCE,
    EXISTENCE_REQUIRED, EXISTENCE_REQUIRED_SOURCE, PRECEDENCE_BINDING_ATTRIBUTE,
    PRECEDENCE_REQUIREMENT, PRECEDENCE_REQUIREMENT_SOURCE, PRECEDENCE_TRIGGER,
    PRECEDENCE_TRIGGER_SOURCE,
};
use sentinel_compile::FilterIdentity;
use sentinel_core::absence::AbsenceSpecification;
use sentinel_core::evaluator::Evaluator;
use sentinel_core::existence::ExistenceSpecification;
use sentinel_core::specification::PrecedenceSpecification;
use sentinel_model::{AttributeKey, AttributeValue, SourceTier};

use crate::error::AdapterError;
use crate::event::{AgentId, LedgerKind};
use crate::source::{
    agent_source, capabilities, event_kind, exchange_source, schema_version, AGENT_ATTRIBUTE,
    BILL_ATTRIBUTE, ORDER_ATTRIBUTE, TAINT_INSTRUCTS_ATTRIBUTE, TAINT_YES,
};

/// Absence-after: no second payment on a settled bill before a credit.
pub const SPEC_ABSENCE: &str = "ledger.bill.no-second-payment";
/// Precedence: a bill's approval is in the causal past of its payment.
pub const SPEC_PRECEDENCE: &str = "ledger.bill.approval-before-payment";
/// Response: an order is filled or cancelled before the deadline.
pub const SPEC_RESPONSE: &str = "ledger.order.fill-or-cancel";
/// A payment whose memo instructs payment needs a prior approval.
pub const SPEC_TAINT: &str = "ledger.bill.tainted-memo-needs-approval";

const SPEC_VERSION: &str = "v1";

/// Per-agent absence evaluator. The agent's pay opens the interval.
///
/// # Errors
///
/// Returns an error when the Pattern cannot be instantiated or lowered.
pub fn absence_evaluator(agent: &AgentId) -> Result<Evaluator, AdapterError> {
    let arguments = BTreeMap::from([
        (
            ABSENCE_BINDING_ATTRIBUTE.to_owned(),
            ParameterValue::AttributeKey(attribute_key(BILL_ATTRIBUTE)),
        ),
        (
            ABSENCE_AFTER.to_owned(),
            selector(LedgerKind::AgentPayBill, Some(agent)),
        ),
        (ABSENCE_UNTIL.to_owned(), selector(LedgerKind::Credit, None)),
        (
            ABSENCE_VETO.to_owned(),
            selector(LedgerKind::ExtraPayment, None),
        ),
    ]);
    let exchange = exchange_source();
    let capable = capabilities();
    let evidence = BTreeMap::from([
        (
            ABSENCE_AFTER_SOURCE.to_owned(),
            EvidenceBinding::new(
                agent_source(agent)?,
                SourceTier::Contributing,
                capable.clone(),
            ),
        ),
        (
            ABSENCE_UNTIL_SOURCE.to_owned(),
            EvidenceBinding::new(exchange.clone(), SourceTier::Anchor, capable.clone()),
        ),
        (
            ABSENCE_VETO_SOURCE.to_owned(),
            EvidenceBinding::new(exchange, SourceTier::Anchor, capable),
        ),
    ]);
    let instance = instantiate(
        PatternId::AbsenceAfterUntil,
        identity(SPEC_ABSENCE),
        ProducerContract::new(schema_version()),
        arguments,
        evidence,
    )?;
    let Some(absence) = instance.ir().as_absence() else {
        return Err(AdapterError::NotAbsence);
    };
    let specification = AbsenceSpecification::try_from(absence)?;
    Ok(Evaluator::absence(specification))
}

/// Per-agent precedence evaluator. Approval must precede that agent's payment.
///
/// The approval event may come from another agent. The exchange is the anchor
/// whose silence, once complete, settles a missing approval as `Violate`.
///
/// # Errors
///
/// Returns an error when the Pattern cannot be instantiated or lowered.
pub fn precedence_evaluator(agent: &AgentId) -> Result<Evaluator, AdapterError> {
    let arguments = BTreeMap::from([
        (
            PRECEDENCE_BINDING_ATTRIBUTE.to_owned(),
            ParameterValue::AttributeKey(attribute_key(BILL_ATTRIBUTE)),
        ),
        (
            PRECEDENCE_TRIGGER.to_owned(),
            selector(LedgerKind::AgentPayBill, Some(agent)),
        ),
        (
            PRECEDENCE_REQUIREMENT.to_owned(),
            selector(LedgerKind::Approval, None),
        ),
    ]);
    let capable = capabilities();
    let evidence = BTreeMap::from([
        (
            PRECEDENCE_TRIGGER_SOURCE.to_owned(),
            EvidenceBinding::new(
                agent_source(agent)?,
                SourceTier::Contributing,
                capable.clone(),
            ),
        ),
        (
            PRECEDENCE_REQUIREMENT_SOURCE.to_owned(),
            EvidenceBinding::new(exchange_source(), SourceTier::Anchor, capable),
        ),
    ]);
    let instance = instantiate(
        PatternId::PrecedenceApproval,
        identity(SPEC_PRECEDENCE),
        ProducerContract::new(schema_version()),
        arguments,
        evidence,
    )?;
    let Some(precedence) = instance.ir().as_precedence() else {
        return Err(AdapterError::NotPrecedence);
    };
    let specification = PrecedenceSpecification::try_from(precedence)?;
    Ok(Evaluator::new(specification))
}

/// Per-agent precedence for a memo that instructs payment.
///
/// The trigger is a payment carrying [`TAINT_INSTRUCTS_ATTRIBUTE`] = [`TAINT_YES`].
/// Code sets that flag from the panel probability. A benign memo does not carry
/// it, so this spec does not fire. The requirement is an approval in the
/// payment's causal past.
///
/// # Errors
///
/// Returns an error when the Pattern cannot be instantiated or lowered.
pub fn tainted_memo_evaluator(agent: &AgentId) -> Result<Evaluator, AdapterError> {
    let mut required = BTreeMap::new();
    required.insert(
        attribute_key(AGENT_ATTRIBUTE),
        AttributeValue::String(agent.as_str().to_owned()),
    );
    required.insert(
        attribute_key(TAINT_INSTRUCTS_ATTRIBUTE),
        AttributeValue::String(TAINT_YES.to_owned()),
    );
    let arguments = BTreeMap::from([
        (
            PRECEDENCE_BINDING_ATTRIBUTE.to_owned(),
            ParameterValue::AttributeKey(attribute_key(BILL_ATTRIBUTE)),
        ),
        (
            PRECEDENCE_TRIGGER.to_owned(),
            ParameterValue::Selector(SelectorValue::new(
                event_kind(LedgerKind::AgentPayBill),
                required,
            )),
        ),
        (
            PRECEDENCE_REQUIREMENT.to_owned(),
            selector(LedgerKind::Approval, None),
        ),
    ]);
    let capable = capabilities();
    let evidence = BTreeMap::from([
        (
            PRECEDENCE_TRIGGER_SOURCE.to_owned(),
            EvidenceBinding::new(
                agent_source(agent)?,
                SourceTier::Contributing,
                capable.clone(),
            ),
        ),
        (
            PRECEDENCE_REQUIREMENT_SOURCE.to_owned(),
            EvidenceBinding::new(exchange_source(), SourceTier::Anchor, capable),
        ),
    ]);
    let instance = instantiate(
        PatternId::PrecedenceApproval,
        identity(SPEC_TAINT),
        ProducerContract::new(schema_version()),
        arguments,
        evidence,
    )?;
    let Some(precedence) = instance.ir().as_precedence() else {
        return Err(AdapterError::NotPrecedence);
    };
    let specification = PrecedenceSpecification::try_from(precedence)?;
    Ok(Evaluator::new(specification))
}

/// Response evaluator. An order must be filled or cancelled before the deadline.
///
/// Open, required, and close are all exchange events, so one anchor source
/// covers the scope.
///
/// # Errors
///
/// Returns an error when the Pattern cannot be instantiated or lowered.
pub fn response_evaluator() -> Result<Evaluator, AdapterError> {
    let arguments = BTreeMap::from([
        (
            EXISTENCE_BINDING_ATTRIBUTE.to_owned(),
            ParameterValue::AttributeKey(attribute_key(ORDER_ATTRIBUTE)),
        ),
        (
            EXISTENCE_OPEN.to_owned(),
            selector(LedgerKind::OrderSubmit, None),
        ),
        (
            EXISTENCE_REQUIRED.to_owned(),
            selector(LedgerKind::OrderResolved, None),
        ),
        (
            EXISTENCE_CLOSE.to_owned(),
            selector(LedgerKind::OrderDeadline, None),
        ),
    ]);
    let exchange = exchange_source();
    let capable = capabilities();
    let evidence = BTreeMap::from([
        (
            EXISTENCE_OPEN_SOURCE.to_owned(),
            EvidenceBinding::new(exchange.clone(), SourceTier::Anchor, capable.clone()),
        ),
        (
            EXISTENCE_REQUIRED_SOURCE.to_owned(),
            EvidenceBinding::new(exchange.clone(), SourceTier::Anchor, capable.clone()),
        ),
        (
            EXISTENCE_CLOSE_SOURCE.to_owned(),
            EvidenceBinding::new(exchange, SourceTier::Contributing, capable),
        ),
    ]);
    let instance = instantiate(
        PatternId::ExistenceBeforeScopeClose,
        identity(SPEC_RESPONSE),
        ProducerContract::new(schema_version()),
        arguments,
        evidence,
    )?;
    let Some(existence) = instance.ir().as_existence() else {
        return Err(AdapterError::NotExistence);
    };
    let specification = ExistenceSpecification::try_from(existence)?;
    Ok(Evaluator::existence(specification))
}

fn identity(specification_id: &str) -> FilterIdentity {
    FilterIdentity {
        specification_id: specification_id.to_owned(),
        specification_version: SPEC_VERSION.to_owned(),
    }
}

fn selector(kind: LedgerKind, agent: Option<&AgentId>) -> ParameterValue {
    let mut required = BTreeMap::new();
    if let Some(agent) = agent {
        required.insert(
            attribute_key(AGENT_ATTRIBUTE),
            AttributeValue::String(agent.as_str().to_owned()),
        );
    }
    ParameterValue::Selector(SelectorValue::new(event_kind(kind), required))
}

fn attribute_key(name: &'static str) -> AttributeKey {
    AttributeKey::new(name).expect("constant attribute key is non-blank")
}
