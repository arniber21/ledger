//! Per-bill absence-after monitor on Sentinel's incremental evaluator.
//!
//! After an agent's `pay_bill` tool call, no further exchange payment may occur
//! before a credit closes that bill's interval. The exchange is the anchor
//! source for the veto and the close. The agent's tool-call log is the
//! contributing source that opens the interval. Judgment calls
//! [`Evaluator::step`] and then [`Evaluator::seal`]. It does not call the
//! batch `evaluate` path.

use std::collections::BTreeMap;

use sentinel_compile::catalogue::{
    instantiate, EvidenceBinding, ParameterValue, PatternId, ProducerContract, SelectorValue,
    ABSENCE_AFTER, ABSENCE_AFTER_SOURCE, ABSENCE_BINDING_ATTRIBUTE, ABSENCE_UNTIL,
    ABSENCE_UNTIL_SOURCE, ABSENCE_VETO, ABSENCE_VETO_SOURCE,
};
use sentinel_compile::FilterIdentity;
use sentinel_core::absence::AbsenceSpecification;
use sentinel_core::evaluator::{Evaluator, Input};
use sentinel_core::verdict::BindingVerdict;
use sentinel_model::{AttributeKey, AttributeValue, SourceTier};

use crate::adapt::{adapt_trace, WithheldSource};
use crate::error::AdapterError;
use crate::event::{AgentId, LedgerEvent, LedgerKind};
use crate::source::{
    agent_source, capabilities, event_kind, exchange_source, schema_version, AGENT_ATTRIBUTE,
    BILL_ATTRIBUTE,
};

const SPEC_ID: &str = "ledger.bill.no-second-payment";
const SPEC_VERSION: &str = "v1";

/// Judges one agent's per-bill absence-after specification over `events`.
///
/// `withheld` drops that source's completeness signal. The events are still
/// stepped. A closed evidence gate yields `Inconclusive`, including when the
/// same trace would be `Satisfy` with the signal present.
///
/// # Errors
///
/// Returns an error when the trace cannot be adapted, the Pattern cannot be
/// instantiated, or the evaluator rejects an input.
pub fn judge_bill_absence(
    agent: &AgentId,
    events: &[LedgerEvent],
    withheld: Option<WithheldSource>,
) -> Result<Vec<BindingVerdict>, AdapterError> {
    let (adapted_events, evidence) = adapt_trace(agent, events, withheld)?.into_parts();
    let mut evaluator = Evaluator::absence(bill_absence(agent)?);
    for event in adapted_events {
        evaluator.step(Input::event(event))?;
    }
    for source in evidence {
        evaluator.step(Input::source(source))?;
    }
    Ok(evaluator.seal()?.into_verdicts())
}

fn bill_absence(agent: &AgentId) -> Result<AbsenceSpecification, AdapterError> {
    let arguments = BTreeMap::from([
        (
            ABSENCE_BINDING_ATTRIBUTE.to_owned(),
            ParameterValue::AttributeKey(binding_attribute()),
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
        FilterIdentity {
            specification_id: SPEC_ID.to_owned(),
            specification_version: SPEC_VERSION.to_owned(),
        },
        ProducerContract::new(schema_version()),
        arguments,
        evidence,
    )?;
    let Some(absence) = instance.ir().as_absence() else {
        return Err(AdapterError::NotAbsence);
    };
    Ok(AbsenceSpecification::try_from(absence)?)
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

fn binding_attribute() -> AttributeKey {
    attribute_key(BILL_ATTRIBUTE)
}

fn attribute_key(name: &'static str) -> AttributeKey {
    AttributeKey::new(name).expect("constant attribute key is non-blank")
}
