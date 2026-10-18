//! Per-bill absence-after monitor on Sentinel's incremental evaluator.
//!
//! After an agent's `pay_bill` tool call, no further exchange payment may occur
//! before a credit closes that bill's interval. The exchange is the anchor
//! source for the veto and the close. The agent's tool-call log is the
//! contributing source that opens the interval. Judgment calls
//! [`Evaluator::step`] and then [`Evaluator::seal`]. It does not call the
//! batch `evaluate` path.

use sentinel_core::evaluator::{Evaluator, Input};
use sentinel_core::verdict::BindingVerdict;

use crate::adapt::{adapt_trace, WithheldSource};
use crate::error::AdapterError;
use crate::event::{AgentId, LedgerEvent};
use crate::specs::absence_evaluator;

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
    let mut evaluator: Evaluator = absence_evaluator(agent)?;
    for event in adapted_events {
        evaluator.step(Input::event(event))?;
    }
    for source in evidence {
        evaluator.step(Input::source(source))?;
    }
    Ok(evaluator.seal()?.into_verdicts())
}
