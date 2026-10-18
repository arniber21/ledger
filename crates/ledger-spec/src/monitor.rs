//! Turn a catalogue-valid draft into a running Sentinel monitor.
//!
//! Evidence roles are bound to the ledger's own sources. An agent event kind
//! is scoped to that agent's tool-call log. An exchange event kind is the
//! exchange source. Invalid drafts do not start a monitor.

use std::collections::BTreeMap;

use ledger_core::{
    adapt_trace, agent_source, capabilities, event_kind, exchange_source, schema_version, tier_of,
    AgentId, LedgerEvent, LedgerKind, AGENT_ATTRIBUTE, BILL_ATTRIBUTE,
};
use sentinel_compile::catalogue::{catalogue, InstanceError};
use sentinel_compile::catalogue::{
    instantiate, EvidenceBinding, ParameterDescriptor, ParameterKind, ParameterValue,
    PatternDeclaration, PatternId, ProducerContract, SelectorValue, ABSENCE_AFTER,
    ABSENCE_AFTER_SOURCE, ABSENCE_UNTIL, ABSENCE_UNTIL_SOURCE, ABSENCE_VETO, ABSENCE_VETO_SOURCE,
    AGGREGATION_MEMBER, AGGREGATION_MEMBER_SOURCE, AGGREGATION_TRIGGER, AGGREGATION_TRIGGER_SOURCE,
    EXISTENCE_CLOSE, EXISTENCE_CLOSE_SOURCE, EXISTENCE_OPEN, EXISTENCE_OPEN_SOURCE,
    EXISTENCE_REQUIRED, EXISTENCE_REQUIRED_SOURCE, PRECEDENCE_REQUIREMENT,
    PRECEDENCE_REQUIREMENT_SOURCE, PRECEDENCE_TRIGGER, PRECEDENCE_TRIGGER_SOURCE, TAINT_ACTION,
    TAINT_ACTION_SOURCE, TAINT_CONTENT_SOURCE, TAINT_UNTRUSTED_CONTENT,
};
use sentinel_compile::FilterIdentity;
use sentinel_core::evaluator::{Evaluator, Input};
use sentinel_core::verdict::BindingVerdict as MonitorVerdict;
use sentinel_core::CoreError;
use sentinel_model::{AttributeKey, AttributeValue};
use thiserror::Error;

use crate::draft::{ledger_kinds, DraftValue, PatternDraft};
use crate::reason::{CatalogueCode, RejectReason};

/// A catalogue Pattern instantiated on the ledger's sources and stepping.
pub struct RunningMonitor {
    pattern_id: String,
    evaluator: Evaluator,
}

impl std::fmt::Debug for RunningMonitor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RunningMonitor")
            .field("pattern_id", &self.pattern_id)
            .finish_non_exhaustive()
    }
}

impl RunningMonitor {
    /// Returns the catalogue id this monitor is running.
    #[must_use]
    pub fn pattern_id(&self) -> &str {
        &self.pattern_id
    }

    /// Steps one adapted input.
    ///
    /// # Errors
    ///
    /// Returns the evaluator's error when the input is refused.
    pub fn step(&mut self, input: Input) -> Result<(), CoreError> {
        self.evaluator.step(input).map(|_| ())
    }

    /// Adapts `events` for `agent`, steps them, and seals.
    ///
    /// # Errors
    ///
    /// Returns an error when the trace cannot be adapted or the evaluator
    /// refuses an input or the seal.
    pub fn judge(
        self,
        agent: &AgentId,
        events: &[LedgerEvent],
    ) -> Result<Vec<MonitorVerdict>, MonitorError> {
        let adapted = adapt_trace(agent, events, None)?;
        let mut evaluator = self.evaluator;
        for event in adapted.events() {
            evaluator.step(Input::event(event.clone()))?;
        }
        for source in adapted.evidence() {
            evaluator.step(Input::source(source.clone()))?;
        }
        Ok(evaluator.seal()?.into_verdicts())
    }
}

/// Why a running monitor could not judge a trace.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum MonitorError {
    /// The trace could not be adapted.
    #[error(transparent)]
    Adapt(#[from] ledger_core::AdapterError),
    /// The evaluator refused an input or the seal.
    #[error(transparent)]
    Core(#[from] CoreError),
}

pub(crate) fn start_monitor(
    draft: &PatternDraft,
    agent: &AgentId,
) -> Result<RunningMonitor, RejectReason> {
    let Some(declaration) = catalogue()
        .iter()
        .find(|item| item.id.as_str() == draft.pattern_id())
    else {
        return Err(RejectReason::UnknownPattern);
    };
    let arguments = bind_arguments(declaration, draft, agent)?;
    let evidence = bind_evidence(declaration, draft, agent)?;
    let instance = instantiate(
        declaration.id,
        FilterIdentity {
            specification_id: format!("ledger.authored.{}", declaration.id.as_str()),
            specification_version: "v1".to_owned(),
        },
        ProducerContract::new(schema_version()),
        arguments,
        evidence,
    )
    .map_err(reject_instance)?;
    let Some(evaluator) = lower(instance.ir()) else {
        return Err(RejectReason::MonitorFailed);
    };
    Ok(RunningMonitor {
        pattern_id: declaration.id.as_str().to_owned(),
        evaluator,
    })
}

fn bind_arguments(
    declaration: &PatternDeclaration,
    draft: &PatternDraft,
    agent: &AgentId,
) -> Result<BTreeMap<String, ParameterValue>, RejectReason> {
    for name in draft.parameters().keys() {
        if declaration.parameter(name).is_none() {
            return Err(RejectReason::CatalogueInvalid {
                code: CatalogueCode::UnknownParameter { name: name.clone() },
            });
        }
    }
    let mut arguments = BTreeMap::new();
    for parameter in declaration.parameters {
        let Some(value) = draft.parameters().get(parameter.name) else {
            return Err(RejectReason::CatalogueInvalid {
                code: CatalogueCode::MissingParameter {
                    name: parameter.name.to_owned(),
                },
            });
        };
        arguments.insert(
            parameter.name.to_owned(),
            bind_parameter(parameter, value, agent)?,
        );
    }
    Ok(arguments)
}

fn bind_parameter(
    parameter: &ParameterDescriptor,
    value: &DraftValue,
    agent: &AgentId,
) -> Result<ParameterValue, RejectReason> {
    let mismatch = RejectReason::CatalogueInvalid {
        code: CatalogueCode::KindMismatch {
            name: parameter.name.to_owned(),
        },
    };
    match (parameter.kind, value) {
        (ParameterKind::AttributeKey, DraftValue::Text(text)) => {
            if text != BILL_ATTRIBUTE {
                return Err(RejectReason::CatalogueInvalid {
                    code: CatalogueCode::BindingAttributeNotEmitted {
                        name: parameter.name.to_owned(),
                    },
                });
            }
            let key =
                AttributeKey::new(text.as_str()).map_err(|_| RejectReason::CatalogueInvalid {
                    code: CatalogueCode::BindingAttributeNotEmitted {
                        name: parameter.name.to_owned(),
                    },
                })?;
            Ok(ParameterValue::AttributeKey(key))
        }
        (ParameterKind::Selector, DraftValue::Text(text)) => {
            let Some(kind) = ledger_kinds()
                .into_iter()
                .find(|kind| kind.event_kind() == text)
            else {
                return Err(RejectReason::CatalogueInvalid {
                    code: CatalogueCode::EventKindNotEmitted {
                        name: parameter.name.to_owned(),
                        event_kind: text.clone(),
                    },
                });
            };
            Ok(selector_value(kind, agent))
        }
        (ParameterKind::SequenceLookbackBound, DraftValue::Integer(number)) => {
            Ok(ParameterValue::SequenceLookbackBound(*number))
        }
        (ParameterKind::CountThreshold, DraftValue::Integer(number)) => {
            Ok(ParameterValue::CountThreshold(*number))
        }
        _ => Err(mismatch),
    }
}

fn selector_value(kind: LedgerKind, agent: &AgentId) -> ParameterValue {
    let mut required = BTreeMap::new();
    if !kind.is_exchange() {
        let key =
            AttributeKey::new(AGENT_ATTRIBUTE).expect("constant agent attribute is non-blank");
        required.insert(key, AttributeValue::String(agent.as_str().to_owned()));
    }
    ParameterValue::Selector(SelectorValue::new(event_kind(kind), required))
}

fn bind_evidence(
    declaration: &PatternDeclaration,
    draft: &PatternDraft,
    agent: &AgentId,
) -> Result<BTreeMap<String, EvidenceBinding>, RejectReason> {
    let roles = selector_roles(declaration.id);
    let mut evidence = BTreeMap::new();
    for requirement in declaration.evidence {
        let Some(parameter_name) = roles
            .iter()
            .find(|(_, role)| *role == requirement.role)
            .map(|(parameter, _)| *parameter)
        else {
            return Err(RejectReason::MonitorFailed);
        };
        let Some(DraftValue::Text(event_kind_name)) = draft.parameters().get(parameter_name) else {
            return Err(RejectReason::CatalogueInvalid {
                code: CatalogueCode::KindMismatch {
                    name: parameter_name.to_owned(),
                },
            });
        };
        let Some(kind) = ledger_kinds()
            .into_iter()
            .find(|kind| kind.event_kind() == event_kind_name)
        else {
            return Err(RejectReason::CatalogueInvalid {
                code: CatalogueCode::EventKindNotEmitted {
                    name: parameter_name.to_owned(),
                    event_kind: event_kind_name.clone(),
                },
            });
        };
        let source = if kind.is_exchange() {
            exchange_source()
        } else {
            agent_source(agent).map_err(|_| RejectReason::MonitorFailed)?
        };
        evidence.insert(
            requirement.role.to_owned(),
            EvidenceBinding::new(source, tier_of(kind), capabilities()),
        );
    }
    Ok(evidence)
}

fn selector_roles(id: PatternId) -> &'static [(&'static str, &'static str)] {
    match id {
        PatternId::PrecedenceApproval => &[
            (PRECEDENCE_TRIGGER, PRECEDENCE_TRIGGER_SOURCE),
            (PRECEDENCE_REQUIREMENT, PRECEDENCE_REQUIREMENT_SOURCE),
        ],
        PatternId::ProvenanceTaint => &[
            (TAINT_ACTION, TAINT_ACTION_SOURCE),
            (TAINT_UNTRUSTED_CONTENT, TAINT_CONTENT_SOURCE),
        ],
        PatternId::AbsenceAfterUntil => &[
            (ABSENCE_AFTER, ABSENCE_AFTER_SOURCE),
            (ABSENCE_UNTIL, ABSENCE_UNTIL_SOURCE),
            (ABSENCE_VETO, ABSENCE_VETO_SOURCE),
        ],
        PatternId::AggregationCountWithin => &[
            (AGGREGATION_TRIGGER, AGGREGATION_TRIGGER_SOURCE),
            (AGGREGATION_MEMBER, AGGREGATION_MEMBER_SOURCE),
        ],
        PatternId::ExistenceBeforeScopeClose => &[
            (EXISTENCE_OPEN, EXISTENCE_OPEN_SOURCE),
            (EXISTENCE_REQUIRED, EXISTENCE_REQUIRED_SOURCE),
            (EXISTENCE_CLOSE, EXISTENCE_CLOSE_SOURCE),
        ],
        _ => &[],
    }
}

fn lower(ir: &sentinel_compile::ir::MonitorIr) -> Option<Evaluator> {
    if let Some(ir) = ir.as_precedence() {
        return Evaluator::try_from(ir).ok();
    }
    if let Some(ir) = ir.as_taint() {
        return Evaluator::try_from(ir).ok();
    }
    if let Some(ir) = ir.as_absence() {
        return Evaluator::try_from(ir).ok();
    }
    if let Some(ir) = ir.as_aggregate() {
        return Evaluator::try_from(ir).ok();
    }
    if let Some(ir) = ir.as_existence() {
        return Evaluator::try_from(ir).ok();
    }
    None
}

fn reject_instance(error: InstanceError) -> RejectReason {
    match error {
        InstanceError::MissingParameter { parameter, .. } => RejectReason::CatalogueInvalid {
            code: CatalogueCode::MissingParameter {
                name: parameter.to_owned(),
            },
        },
        InstanceError::UnknownParameter { parameter, .. } => RejectReason::CatalogueInvalid {
            code: CatalogueCode::UnknownParameter { name: parameter },
        },
        InstanceError::ParameterKindMismatch { parameter, .. } => RejectReason::CatalogueInvalid {
            code: CatalogueCode::KindMismatch {
                name: parameter.to_owned(),
            },
        },
        _ => RejectReason::MonitorFailed,
    }
}
