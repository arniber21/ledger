//! Live monitors. Each new ledger event is stepped once. A snapshot clones the
//! evaluators, applies completeness, and seals the clones. The live evaluators
//! stay open for the next event.

use std::collections::BTreeMap;

use sentinel_core::evaluator::{Evaluator, Input};
use sentinel_core::verdict::BindingVerdict;
use sentinel_core::CoreError;
use sentinel_model::{Event, InconclusiveReason, VerdictOutcome};

use crate::adapt::TraceCursor;
use crate::error::AdapterError;
use crate::event::{AgentId, LedgerEvent, LedgerKind};
use crate::gc::MonitorDesk;
use crate::specs::{
    absence_evaluator, precedence_evaluator, response_evaluator, SPEC_ABSENCE, SPEC_PRECEDENCE,
    SPEC_RESPONSE,
};

/// Ticks after an order before the response scope closes.
pub const RESPONSE_DEADLINE_TICKS: u64 = 3;

/// One judged binding, with the evidence timeline that produced it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Judged {
    /// Specification id (`ledger.bill.no-second-payment` and the others).
    pub spec_id: String,
    /// Bill id or order id.
    pub binding: String,
    /// `Satisfy`, `Violate`, or `Inconclusive`.
    pub outcome: VerdictOutcome,
    /// Stable reason code. Never model prose.
    pub reason_code: Option<String>,
    /// Evidence events in observation order.
    pub timeline: Vec<TimelineEntry>,
}

/// One evidence event on a counterexample timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimelineEntry {
    /// Ledger event id.
    pub event_id: String,
    /// Observation time in nanoseconds.
    pub observed_at_ns: i128,
    /// `exchange` or `agent.{id}`.
    pub source: String,
    /// Sentinel event kind.
    pub kind: String,
    /// Role such as `payment`, `approval`, or `fill`.
    pub role: String,
}

struct AgentMonitors {
    absence: Evaluator,
    precedence: Evaluator,
}

struct OpenOrder {
    opened_on_tick: u64,
    last_id: String,
    agent: AgentId,
    closed: bool,
}

/// Incremental desk: absence and precedence per agent, response for orders.
pub struct Session {
    cursor: TraceCursor,
    history: Vec<LedgerEvent>,
    adapted: Vec<Event>,
    agents: BTreeMap<String, AgentMonitors>,
    response: Evaluator,
    orders: BTreeMap<String, OpenOrder>,
    tick_count: u64,
    desk: MonitorDesk,
}

impl Session {
    /// Starts the response monitor. Agent monitors start on the first agent event.
    ///
    /// # Errors
    ///
    /// Returns an error when the response Pattern cannot be instantiated.
    pub fn new() -> Result<Self, AdapterError> {
        Ok(Self {
            cursor: TraceCursor::new(),
            history: Vec::new(),
            adapted: Vec::new(),
            agents: BTreeMap::new(),
            response: response_evaluator()?,
            orders: BTreeMap::new(),
            tick_count: 0,
            desk: MonitorDesk::new()?,
        })
    }

    /// How many per-bill monitors are still watching.
    ///
    /// Paid-in-full, a passed horizon, and an affirmed ambiguous close remove
    /// a monitor. Unknown evidence does not.
    #[must_use]
    pub fn live_monitor_count(&self) -> usize {
        self.desk.live_count()
    }

    /// The retirement desk for this session.
    #[must_use]
    pub fn monitors(&mut self) -> &mut MonitorDesk {
        &mut self.desk
    }

    /// Steps one event into every running monitor.
    ///
    /// A second payment on a bill is judged as the absence veto. An extra
    /// payment also closes that bill's interval. Three ticks after an order
    /// close the response scope.
    ///
    /// # Errors
    ///
    /// Returns an error when the event cannot be adapted or an evaluator
    /// rejects it.
    pub fn push(&mut self, event: LedgerEvent) -> Result<(), AdapterError> {
        let event = self.classify(event)?;
        self.push_inner(event, true)
    }

    /// Seals clones of the running monitors.
    ///
    /// Missing evidence is `Inconclusive`. A binding with no sealed verdict is
    /// not reported as `Satisfy`.
    ///
    /// # Errors
    ///
    /// Returns an error when a clone rejects completeness or the seal, other
    /// than an empty evidence gate.
    pub fn verdicts(&self) -> Result<Vec<Judged>, AdapterError> {
        let evidence = self.cursor.evidence()?;
        let mut judged = Vec::new();
        for monitor in self.agents.values() {
            judged.extend(seal(
                &monitor.absence,
                &evidence,
                SPEC_ABSENCE,
                &self.history,
            )?);
            judged.extend(seal(
                &monitor.precedence,
                &evidence,
                SPEC_PRECEDENCE,
                &self.history,
            )?);
        }
        judged.extend(seal(
            &self.response,
            &evidence,
            SPEC_RESPONSE,
            &self.history,
        )?);
        judged.extend(self.gaps(&judged));
        Ok(strongest(judged))
    }

    fn classify(&self, event: LedgerEvent) -> Result<LedgerEvent, AdapterError> {
        if event.kind() != LedgerKind::AgentPayBill {
            return Ok(event);
        }
        let Some(bill) = event.bill_id().map(|bill| bill.as_str().to_owned()) else {
            return Ok(event);
        };
        let already_paid = self.history.iter().any(|prior| {
            prior.kind() == LedgerKind::AgentPayBill
                && prior.bill_id().is_some_and(|id| id.as_str() == bill)
        });
        if !already_paid {
            return Ok(event);
        }
        let mut extra = event.recast(LedgerKind::ExtraPayment)?;
        if extra.predecessors().is_empty() {
            if let Some(first) = self.history.iter().find(|prior| {
                prior.kind() == LedgerKind::AgentPayBill
                    && prior.bill_id().is_some_and(|id| id.as_str() == bill)
            }) {
                extra.precede_by(first.id())?;
            }
        }
        Ok(extra)
    }

    fn push_inner(&mut self, event: LedgerEvent, synthesize: bool) -> Result<(), AdapterError> {
        let adapted = self.cursor.push(&event)?;
        self.response.step(Input::event(adapted.clone()))?;
        for monitor in self.agents.values_mut() {
            monitor.step(&adapted)?;
        }
        let create_agent =
            !event.kind().is_exchange() && !self.agents.contains_key(event.agent_id().as_str());
        self.note_order(&event);
        self.history.push(event.clone());
        self.adapted.push(adapted);
        self.desk.follow(&event)?;
        if create_agent {
            let mut monitor = AgentMonitors::new(event.agent_id())?;
            for prior in &self.adapted {
                monitor.step(prior)?;
            }
            self.agents
                .insert(event.agent_id().as_str().to_owned(), monitor);
        }
        if !synthesize {
            return Ok(());
        }
        if event.kind() == LedgerKind::ExtraPayment {
            let close = close_for(&event)?;
            self.push_inner(close, false)?;
        }
        if event.kind() == LedgerKind::Tick {
            self.tick_count = self.tick_count.saturating_add(1);
            let deadlines = self.deadlines(&event)?;
            for deadline in deadlines {
                self.push_inner(deadline, false)?;
            }
        }
        Ok(())
    }

    fn note_order(&mut self, event: &LedgerEvent) {
        let Some(order_id) = event.order_id().map(|id| id.as_str().to_owned()) else {
            return;
        };
        match event.kind() {
            LedgerKind::OrderSubmit => {
                self.orders.entry(order_id).or_insert(OpenOrder {
                    opened_on_tick: self.tick_count,
                    last_id: event.id().to_owned(),
                    agent: event.agent_id().clone(),
                    closed: false,
                });
            }
            LedgerKind::OrderResolved => {
                if let Some(order) = self.orders.get_mut(&order_id) {
                    order.last_id = event.id().to_owned();
                }
            }
            LedgerKind::OrderDeadline => {
                if let Some(order) = self.orders.get_mut(&order_id) {
                    order.closed = true;
                }
            }
            _ => {}
        }
    }

    fn deadlines(&self, tick: &LedgerEvent) -> Result<Vec<LedgerEvent>, AdapterError> {
        let mut due = Vec::new();
        for (order_id, order) in &self.orders {
            if order.closed {
                continue;
            }
            let elapsed = self.tick_count.saturating_sub(order.opened_on_tick);
            if elapsed < RESPONSE_DEADLINE_TICKS {
                continue;
            }
            let times = tick.times();
            let event = LedgerEvent::order(
                format!("deadline-{order_id}"),
                LedgerKind::OrderDeadline,
                crate::event::OrderId::new(order_id)?,
                None,
                order.agent.clone(),
                crate::event::Times::new(
                    times.occurred_at_ns.saturating_add(1),
                    times.observed_at_ns.saturating_add(1),
                ),
                [order.last_id.as_str()],
            )?
            .derived();
            due.push(event);
        }
        Ok(due)
    }

    fn gaps(&self, judged: &[Judged]) -> Vec<Judged> {
        let mut gaps = Vec::new();
        for event in &self.history {
            if event.kind() == LedgerKind::AgentPayBill {
                let Some(binding) = event.bill_id().map(|id| id.as_str().to_owned()) else {
                    continue;
                };
                push_gap(&mut gaps, judged, SPEC_ABSENCE, &binding, &self.history);
                push_gap(&mut gaps, judged, SPEC_PRECEDENCE, &binding, &self.history);
            }
            if event.kind() == LedgerKind::OrderSubmit {
                let Some(binding) = event.order_id().map(|id| id.as_str().to_owned()) else {
                    continue;
                };
                push_gap(&mut gaps, judged, SPEC_RESPONSE, &binding, &self.history);
            }
        }
        gaps
    }
}

impl AgentMonitors {
    fn new(agent: &AgentId) -> Result<Self, AdapterError> {
        Ok(Self {
            absence: absence_evaluator(agent)?,
            precedence: precedence_evaluator(agent)?,
        })
    }

    fn step(&mut self, event: &Event) -> Result<(), AdapterError> {
        self.absence.step(Input::event(event.clone()))?;
        self.precedence.step(Input::event(event.clone()))?;
        Ok(())
    }
}

fn close_for(extra: &LedgerEvent) -> Result<LedgerEvent, AdapterError> {
    let Some(bill) = extra.bill_id().cloned() else {
        return Err(AdapterError::KindMismatch {
            kind: extra.kind().event_kind(),
            subject: "bill",
        });
    };
    let times = extra.times();
    Ok(LedgerEvent::new(
        format!("{}:close", extra.id()),
        LedgerKind::Credit,
        bill,
        extra.agent_id().clone(),
        crate::event::Times::new(
            times.occurred_at_ns.saturating_add(1),
            times.observed_at_ns.saturating_add(1),
        ),
        [extra.id()],
    )?
    .derived())
}

fn seal(
    evaluator: &Evaluator,
    evidence: &[sentinel_model::SourceEvidence],
    spec_id: &str,
    history: &[LedgerEvent],
) -> Result<Vec<Judged>, AdapterError> {
    let mut clone = evaluator.clone();
    for source in evidence {
        clone.step(Input::source(source.clone()))?;
    }
    let verdicts = match clone.seal() {
        Ok(sealed) => sealed.into_verdicts(),
        Err(CoreError::NoEvidenceCoverage { .. }) => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    Ok(verdicts
        .into_iter()
        .map(|verdict| judged_from(spec_id, verdict, history))
        .collect())
}

fn judged_from(spec_id: &str, verdict: BindingVerdict, history: &[LedgerEvent]) -> Judged {
    let outcome = verdict.verdict().outcome();
    let reason_code = reason_code(spec_id, outcome, verdict.verdict().inconclusive_reason());
    let binding = verdict.binding().to_owned();
    Judged {
        spec_id: spec_id.to_owned(),
        binding: binding.clone(),
        outcome,
        reason_code,
        timeline: timeline(spec_id, &binding, history),
    }
}

fn reason_code(
    spec_id: &str,
    outcome: VerdictOutcome,
    inconclusive: Option<InconclusiveReason>,
) -> Option<String> {
    match outcome {
        VerdictOutcome::Satisfy => None,
        VerdictOutcome::Violate => Some(
            match spec_id {
                SPEC_ABSENCE => "second-payment",
                SPEC_PRECEDENCE => "payment-without-approval",
                SPEC_RESPONSE => "order-unresolved",
                _ => "needs-review",
            }
            .to_owned(),
        ),
        VerdictOutcome::Inconclusive | _ => Some(
            match inconclusive {
                Some(InconclusiveReason::PrefixUncertainty) => "prefix-uncertainty",
                Some(InconclusiveReason::ObservationalUncertainty) => "observational-uncertainty",
                Some(InconclusiveReason::EpistemicUncertainty) => "epistemic-uncertainty",
                Some(InconclusiveReason::AnchorSilence) => "anchor-silence",
                None | Some(_) => "missing-evidence",
            }
            .to_owned(),
        ),
    }
}

fn push_gap(
    gaps: &mut Vec<Judged>,
    judged: &[Judged],
    spec_id: &str,
    binding: &str,
    history: &[LedgerEvent],
) {
    let covered = judged
        .iter()
        .chain(gaps.iter())
        .any(|item| item.spec_id == spec_id && item.binding == binding);
    if covered {
        return;
    }
    gaps.push(Judged {
        spec_id: spec_id.to_owned(),
        binding: binding.to_owned(),
        outcome: VerdictOutcome::Inconclusive,
        reason_code: Some("missing-evidence".to_owned()),
        timeline: timeline(spec_id, binding, history),
    });
}

fn timeline(spec_id: &str, binding: &str, history: &[LedgerEvent]) -> Vec<TimelineEntry> {
    let mut rows: Vec<&LedgerEvent> = history
        .iter()
        .filter(|event| on_timeline(spec_id, binding, event, history))
        .collect();
    rows.sort_by_key(|event| (event.times().observed_at_ns, event.id().to_owned()));
    rows.into_iter()
        .map(|event| TimelineEntry {
            event_id: event.id().to_owned(),
            observed_at_ns: event.times().observed_at_ns,
            source: event.source_name(),
            kind: event.kind().event_kind().to_owned(),
            role: event.role().to_owned(),
        })
        .collect()
}

fn on_timeline(spec_id: &str, binding: &str, event: &LedgerEvent, history: &[LedgerEvent]) -> bool {
    if spec_id == SPEC_RESPONSE {
        if event.order_id().is_some_and(|id| id.as_str() == binding) {
            return true;
        }
        return event.kind() == LedgerKind::Tick && tick_in_order_window(binding, event, history);
    }
    if event.is_derived() {
        return false;
    }
    event.bill_id().is_some_and(|id| id.as_str() == binding)
}

fn tick_in_order_window(binding: &str, tick: &LedgerEvent, history: &[LedgerEvent]) -> bool {
    let Some(open) = history.iter().find(|event| {
        event.kind() == LedgerKind::OrderSubmit
            && event.order_id().is_some_and(|id| id.as_str() == binding)
    }) else {
        return false;
    };
    let start = open.times().observed_at_ns;
    let end = history
        .iter()
        .filter(|event| {
            event.kind() == LedgerKind::OrderDeadline
                && event.order_id().is_some_and(|id| id.as_str() == binding)
        })
        .map(|event| event.times().observed_at_ns)
        .max();
    let at = tick.times().observed_at_ns;
    at >= start && end.is_none_or(|end| at <= end)
}

fn strongest(judged: Vec<Judged>) -> Vec<Judged> {
    let mut best: BTreeMap<(String, String), Judged> = BTreeMap::new();
    for item in judged {
        let key = (item.spec_id.clone(), item.binding.clone());
        match best.get(&key) {
            Some(current) if rank(current.outcome) >= rank(item.outcome) => {}
            _ => {
                best.insert(key, item);
            }
        }
    }
    best.into_values().collect()
}

fn rank(outcome: VerdictOutcome) -> u8 {
    match outcome {
        VerdictOutcome::Violate => 2,
        VerdictOutcome::Inconclusive => 1,
        VerdictOutcome::Satisfy | _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{BillId, OrderEnd, OrderId, Times};

    fn agent() -> AgentId {
        AgentId::new("payer").expect("agent")
    }

    fn bill(id: &str) -> BillId {
        BillId::new(id).expect("bill")
    }

    fn order(id: &str) -> OrderId {
        OrderId::new(id).expect("order")
    }

    fn pay(id: &str, at: i128, predecessors: &[&str]) -> LedgerEvent {
        LedgerEvent::new(
            id,
            LedgerKind::AgentPayBill,
            bill("bill-1"),
            agent(),
            Times::at(at),
            predecessors.iter().copied(),
        )
        .expect("pay")
    }

    fn exchange(id: &str, kind: LedgerKind, at: i128, predecessors: &[&str]) -> LedgerEvent {
        LedgerEvent::new(
            id,
            kind,
            bill("bill-1"),
            agent(),
            Times::at(at),
            predecessors.iter().copied(),
        )
        .expect("exchange event")
    }

    fn push_all(session: &mut Session, events: &[LedgerEvent]) {
        for event in events {
            session.push(event.clone()).expect("push");
        }
    }

    fn one<'a>(verdicts: &'a [Judged], spec: &str) -> &'a Judged {
        verdicts
            .iter()
            .find(|item| item.spec_id == spec && item.binding == "bill-1")
            .unwrap_or_else(|| panic!("missing {spec} in {verdicts:?}"))
    }

    #[test]
    fn second_payment_violates_with_a_timeline() {
        let mut session = Session::new().expect("session");
        push_all(
            &mut session,
            &[
                pay("pay", 10, &[]),
                exchange("settled", LedgerKind::ObligationSettled, 20, &["pay"]),
                pay("pay-again", 30, &["settled"]),
                exchange("extra", LedgerKind::ExtraPayment, 40, &["pay-again"]),
                exchange("credit", LedgerKind::Credit, 50, &["extra"]),
            ],
        );
        let verdicts = session.verdicts().expect("verdicts");
        let verdict = one(&verdicts, SPEC_ABSENCE);
        assert_eq!(verdict.outcome, VerdictOutcome::Violate);
        assert_eq!(verdict.reason_code.as_deref(), Some("second-payment"));
        assert!(verdict.timeline.len() >= 2, "{:?}", verdict.timeline);
        assert!(verdict
            .timeline
            .windows(2)
            .all(|pair| pair[0].observed_at_ns <= pair[1].observed_at_ns));
        assert!(verdict
            .timeline
            .iter()
            .any(|entry| entry.role == "second-payment"));
        assert!(!format!("{verdict:?}").to_lowercase().contains("fraud"));
    }

    #[test]
    fn one_payment_then_credit_satisfies_absence() {
        let mut session = Session::new().expect("session");
        push_all(
            &mut session,
            &[
                pay("pay", 10, &[]),
                exchange("settled", LedgerKind::ObligationSettled, 20, &["pay"]),
                exchange("credit", LedgerKind::Credit, 30, &["settled"]),
            ],
        );
        let verdicts = session.verdicts().expect("verdicts");
        assert_eq!(
            one(&verdicts, SPEC_ABSENCE).outcome,
            VerdictOutcome::Satisfy
        );
    }

    #[test]
    fn payment_without_approval_violates_when_the_exchange_is_complete() {
        let mut session = Session::new().expect("session");
        push_all(
            &mut session,
            &[
                pay("pay", 10, &[]),
                exchange("settled", LedgerKind::ObligationSettled, 20, &["pay"]),
            ],
        );
        let verdicts = session.verdicts().expect("verdicts");
        let verdict = one(&verdicts, SPEC_PRECEDENCE);
        assert_eq!(verdict.outcome, VerdictOutcome::Violate);
        assert_eq!(
            verdict.reason_code.as_deref(),
            Some("payment-without-approval")
        );
        assert!(!verdict.timeline.is_empty());
    }

    #[test]
    fn approval_before_payment_satisfies_precedence() {
        let mut session = Session::new().expect("session");
        let approval = LedgerEvent::new(
            "approval",
            LedgerKind::Approval,
            bill("bill-1"),
            AgentId::new("approver").expect("approver"),
            Times::at(5),
            std::iter::empty::<&str>(),
        )
        .expect("approval");
        push_all(
            &mut session,
            &[
                approval,
                pay("pay", 10, &["approval"]),
                exchange("settled", LedgerKind::ObligationSettled, 20, &["pay"]),
            ],
        );
        let verdicts = session.verdicts().expect("verdicts");
        assert_eq!(
            one(&verdicts, SPEC_PRECEDENCE).outcome,
            VerdictOutcome::Satisfy
        );
    }

    #[test]
    fn pay_alone_is_inconclusive() {
        let mut session = Session::new().expect("session");
        session.push(pay("pay", 10, &[])).expect("push");
        let verdicts = session.verdicts().expect("verdicts");
        for spec in [SPEC_ABSENCE, SPEC_PRECEDENCE] {
            let verdict = one(&verdicts, spec);
            assert_eq!(verdict.outcome, VerdictOutcome::Inconclusive, "{spec}");
            assert_ne!(verdict.outcome, VerdictOutcome::Satisfy);
        }
    }

    #[test]
    fn order_without_fill_within_three_ticks_violates() {
        let mut session = Session::new().expect("session");
        let buyer = AgentId::new("buyer").expect("buyer");
        session
            .push(
                LedgerEvent::order(
                    "order-1",
                    LedgerKind::OrderSubmit,
                    order("order-1"),
                    None,
                    buyer.clone(),
                    Times::at(10),
                    std::iter::empty::<&str>(),
                )
                .expect("order"),
            )
            .expect("push order");
        for index in 1..=3 {
            session
                .push(
                    LedgerEvent::tick(
                        format!("tick-{index}"),
                        buyer.clone(),
                        Times::at(10 + i128::from(index)),
                        std::iter::empty::<&str>(),
                    )
                    .expect("tick"),
                )
                .expect("push tick");
        }
        let verdicts = session.verdicts().expect("verdicts");
        let verdict = verdicts
            .iter()
            .find(|item| item.spec_id == SPEC_RESPONSE && item.binding == "order-1")
            .expect("response verdict");
        assert_eq!(verdict.outcome, VerdictOutcome::Violate);
        assert_eq!(verdict.reason_code.as_deref(), Some("order-unresolved"));
        assert!(verdict.timeline.iter().any(|entry| entry.role == "order"));
        assert!(verdict.timeline.iter().any(|entry| entry.role == "tick"));
        assert!(verdict
            .timeline
            .iter()
            .any(|entry| entry.role == "deadline"));
    }

    #[test]
    fn fill_before_the_deadline_satisfies_response() {
        let mut session = Session::new().expect("session");
        let buyer = AgentId::new("buyer").expect("buyer");
        session
            .push(
                LedgerEvent::order(
                    "order-1",
                    LedgerKind::OrderSubmit,
                    order("order-1"),
                    None,
                    buyer.clone(),
                    Times::at(10),
                    std::iter::empty::<&str>(),
                )
                .expect("order"),
            )
            .expect("push order");
        session
            .push(
                LedgerEvent::order(
                    "fill-1",
                    LedgerKind::OrderResolved,
                    order("order-1"),
                    Some(OrderEnd::Fill),
                    buyer.clone(),
                    Times::at(11),
                    ["order-1"],
                )
                .expect("fill"),
            )
            .expect("push fill");
        for index in 1..=3 {
            session
                .push(
                    LedgerEvent::tick(
                        format!("tick-{index}"),
                        buyer.clone(),
                        Times::at(11 + i128::from(index)),
                        std::iter::empty::<&str>(),
                    )
                    .expect("tick"),
                )
                .expect("push tick");
        }
        let verdicts = session.verdicts().expect("verdicts");
        let verdict = verdicts
            .iter()
            .find(|item| item.spec_id == SPEC_RESPONSE)
            .expect("response");
        assert_eq!(verdict.outcome, VerdictOutcome::Satisfy);
        assert!(verdict.timeline.iter().any(|entry| entry.role == "fill"));
    }

    #[test]
    fn live_monitor_count_tracks_an_open_bill() {
        let mut session = Session::new().expect("session");
        assert_eq!(session.live_monitor_count(), 0);
        session.push(pay("pay", 10, &[])).expect("pay");
        assert_eq!(session.live_monitor_count(), 1);
        assert!(matches!(
            session
                .monitors()
                .record_payment("bill-1", "1.00", "USD", 10)
                .expect("partial without owed stays open"),
            crate::gc::Decision::Kept { .. }
        ));
        assert_eq!(session.live_monitor_count(), 1);
    }
}
