//! Per-binding monitor retirement.
//!
//! Deterministic closes run first: paid in full, then a horizon that was
//! actually supplied. A credit or cancellation memo, and an alias that might
//! be the same obligation, are ambiguous. Those stay open unless the caller
//! passes [`CloseJudgment::Affirm`]. [`CloseJudgment::Unknown`] keeps the
//! monitor. A missing amount, currency, or horizon does not retire it.
//! Retirement is not a pass: the stored outcome is the absence seal when one
//! exists, and `Inconclusive` when it does not. Say "needs review".

use std::collections::BTreeMap;

use sentinel_core::evaluator::{Evaluator, Input};
use sentinel_core::slice::{Binding, RetirementCause, RetirementSignal, ScopeHorizon};
use sentinel_core::verdict::BindingVerdict as SentinelVerdict;
use sentinel_core::CoreError;
use sentinel_model::{ObservationTime, VerdictOutcome};

use crate::dataset::Money;
use crate::error::AdapterError;
use crate::event::{AgentId, LedgerEvent, LedgerKind, Times};
use crate::source::sentinel_clock;
use crate::specs::{absence_evaluator, response_evaluator};
use crate::TraceCursor;

/// Whether an ambiguous close is strong enough to retire a monitor.
///
/// Callers map Jev through the existing 0.80 thresholds before choosing a
/// variant. This desk does not score probabilities itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub enum CloseJudgment {
    /// A score at or above the fixed threshold says the close is real.
    Affirm,
    /// A score at or above the fixed threshold says the close is not real.
    ///
    /// The monitor stays. This is not a pass.
    Deny,
    /// Timeout, error, a score under the threshold, a contradiction, or
    /// missing evidence. The monitor stays. This is not a pass.
    Unknown,
}

/// What one retirement attempt did.
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use]
pub enum Decision {
    /// The monitor was retired with a Sentinel [`RetirementSignal`].
    Retired {
        /// Monitor key that left the live set.
        key: String,
    },
    /// The monitor is still live, or it was not live to begin with.
    Kept {
        /// Monitor key the attempt named.
        key: String,
    },
}

/// One bill monitor to open.
#[derive(Clone, Debug)]
pub struct OpenBill {
    key: String,
    agent: AgentId,
    opened_at_ns: i128,
    horizon_ns: Option<i128>,
    owed: Option<Money>,
    currency: Option<String>,
}

impl OpenBill {
    /// A monitor with no owed amount and no horizon.
    ///
    /// Neither paid-in-full nor the clock can retire it until the caller adds
    /// those facts.
    #[must_use]
    pub fn new(key: impl Into<String>, agent: AgentId, opened_at_ns: i128) -> Self {
        Self {
            key: key.into(),
            agent,
            opened_at_ns,
            horizon_ns: None,
            owed: None,
            currency: None,
        }
    }

    /// Sets the observation time at which the horizon close becomes due.
    ///
    /// The desk does not read a clock. [`MonitorDesk::advance`] compares this
    /// value to the `as_of` the caller supplies.
    #[must_use]
    pub fn with_horizon(mut self, horizon_ns: i128) -> Self {
        self.horizon_ns = Some(horizon_ns);
        self
    }

    /// Sets the amount and currency a paid-in-full close needs.
    ///
    /// A blank currency is dropped. Paid-in-full then cannot fire.
    #[must_use]
    pub fn with_owed(mut self, owed: Money, currency: impl Into<String>) -> Self {
        let currency = currency.into();
        let currency = currency.trim();
        self.owed = Some(owed);
        self.currency = (!currency.is_empty()).then(|| currency.to_owned());
        self
    }
}

struct Watch {
    evaluator: Evaluator,
    cursor: TraceCursor,
    history: Vec<LedgerEvent>,
}

struct LiveMonitor {
    agent: AgentId,
    opened_at_ns: i128,
    horizon_ns: Option<i128>,
    owed: Option<Money>,
    currency: Option<String>,
    paid: Money,
    watch: Option<Watch>,
}

/// Live per-binding monitors.
///
/// Each open bill is one monitor. [`Self::live_count`] is the size of that
/// set. A retired binding accepts a [`RetirementSignal`] on a deadline
/// specification and then leaves the set. It is not reopened.
pub struct MonitorDesk {
    live: BTreeMap<String, LiveMonitor>,
    closed: BTreeMap<String, VerdictOutcome>,
    scopes: Evaluator,
}

impl MonitorDesk {
    /// Starts the deadline evaluator that admits retirement signals.
    ///
    /// # Errors
    ///
    /// Returns an error when the response Pattern cannot be instantiated.
    pub fn new() -> Result<Self, AdapterError> {
        Ok(Self {
            live: BTreeMap::new(),
            closed: BTreeMap::new(),
            scopes: response_evaluator()?,
        })
    }

    /// How many monitors are still watching.
    #[must_use]
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    /// How many monitors have been retired and left the live set.
    #[must_use]
    pub fn retired_count(&self) -> usize {
        self.closed.len()
    }

    /// Whether `key` is still watching.
    #[must_use]
    pub fn is_live(&self, key: &str) -> bool {
        self.live.contains_key(key)
    }

    /// Opens `bill` unless that key is already live or already retired.
    ///
    /// A retired key stays retired. A second open does not reset owed or the
    /// horizon on a live key.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Blank`] when the key is empty.
    pub fn open_bill(&mut self, bill: OpenBill) -> Result<(), AdapterError> {
        let key = bill.key.trim();
        if key.is_empty() {
            return Err(AdapterError::Blank("monitor key"));
        }
        let key = key.to_owned();
        if self.closed.contains_key(&key) || self.live.contains_key(&key) {
            return Ok(());
        }
        self.live.insert(
            key,
            LiveMonitor {
                agent: bill.agent,
                opened_at_ns: bill.opened_at_ns,
                horizon_ns: bill.horizon_ns,
                owed: bill.owed,
                currency: bill.currency,
                paid: Money::ZERO,
                watch: None,
            },
        );
        Ok(())
    }

    /// Adds a payment. Retires when the summed same-currency amount covers owed.
    ///
    /// An unparsable amount, a negative amount, a currency mismatch, a missing
    /// owed amount, or a missing currency does not retire and does not count
    /// as a pass. Overflow leaves the previous sum in place.
    ///
    /// # Errors
    ///
    /// Returns an error when the retirement signal is rejected.
    pub fn record_payment(
        &mut self,
        key: &str,
        amount: &str,
        currency: &str,
        observed_at_ns: i128,
    ) -> Result<Decision, AdapterError> {
        if !self.is_live(key) {
            return Ok(kept(key));
        }
        let covered = self
            .live
            .get_mut(key)
            .is_some_and(|monitor| payment_covers(monitor, amount, currency));
        if !covered {
            return Ok(kept(key));
        }
        self.retire(key, observed_at_ns)?;
        Ok(retired(key))
    }

    /// Retires every live monitor whose horizon is at or before `as_of_ns`.
    ///
    /// A monitor with no horizon stays. This does not consult Jev.
    ///
    /// # Errors
    ///
    /// Returns an error when a retirement signal is rejected. Monitors already
    /// retired in this call stay retired.
    pub fn advance(&mut self, as_of_ns: i128) -> Result<usize, AdapterError> {
        let due: Vec<String> = self
            .live
            .iter()
            .filter(|(_, monitor)| {
                monitor
                    .horizon_ns
                    .is_some_and(|horizon| as_of_ns >= horizon)
            })
            .map(|(key, _)| key.clone())
            .collect();
        let count = due.len();
        for key in due {
            self.retire(&key, as_of_ns)?;
        }
        Ok(count)
    }

    /// Applies an ambiguous-close judgment.
    ///
    /// Only [`CloseJudgment::Affirm`] retires. Deny and unknown keep the monitor.
    ///
    /// # Errors
    ///
    /// Returns an error when the retirement signal is rejected.
    pub fn judge_close(
        &mut self,
        key: &str,
        judgment: CloseJudgment,
        observed_at_ns: i128,
    ) -> Result<Decision, AdapterError> {
        if judgment != CloseJudgment::Affirm || !self.is_live(key) {
            return Ok(kept(key));
        }
        self.retire(key, observed_at_ns)?;
        Ok(retired(key))
    }

    /// Merges `alias` into `canonical` when `judgment` affirms they are one obligation.
    ///
    /// Unknown and deny leave both keys as they are. An alias that is the
    /// canonical key is already one monitor. When the canonical key is not
    /// live, the alias monitor moves onto that key and keeps watching.
    ///
    /// # Errors
    ///
    /// Returns an error when the retirement signal is rejected. The alias
    /// stays live in that case.
    pub fn apply_alias(
        &mut self,
        alias: &str,
        canonical: &str,
        judgment: CloseJudgment,
        observed_at_ns: i128,
    ) -> Result<Decision, AdapterError> {
        if judgment != CloseJudgment::Affirm || alias == canonical || !self.is_live(alias) {
            return Ok(kept(alias));
        }
        let outcome = self.preview(alias)?;
        self.retire_signal(alias, observed_at_ns)?;
        let monitor = self.live.remove(alias);
        self.closed.insert(alias.to_owned(), outcome);
        if let Some(monitor) = monitor {
            if !self.live.contains_key(canonical) && !self.closed.contains_key(canonical) {
                self.live.insert(canonical.to_owned(), monitor);
            }
        }
        Ok(retired(alias))
    }

    /// Steps `event` into the live monitor for its bill.
    ///
    /// A second `pay_bill` on that bill is the absence veto. The following
    /// credit is the interval close, same as the live session. A retired bill
    /// ignores the event and is not reopened. Events with no bill id are ignored.
    ///
    /// # Errors
    ///
    /// Returns an error when the event cannot be adapted or the absence
    /// evaluator rejects it.
    pub fn push(&mut self, event: LedgerEvent) -> Result<(), AdapterError> {
        let Some(key) = event.bill_id().map(|bill| bill.as_str().to_owned()) else {
            return Ok(());
        };
        if !self.is_live(&key) {
            return Ok(());
        }
        let event = classify(self.history(&key), event)?;
        self.step_live(&event)?;
        if event.kind() == LedgerKind::ExtraPayment {
            self.step_live(&close_for(&event)?)?;
        }
        Ok(())
    }

    /// Steps one already-classified event. Used by the live session.
    ///
    /// Opens a monitor for a new bill with no owed amount and no horizon, so
    /// this path alone does not retire it.
    ///
    /// # Errors
    ///
    /// Returns an error when the event cannot be adapted or the absence
    /// evaluator rejects it.
    pub(crate) fn follow(&mut self, event: &LedgerEvent) -> Result<(), AdapterError> {
        let Some(bill) = event.bill_id() else {
            return Ok(());
        };
        let key = bill.as_str();
        if self.closed.contains_key(key) {
            return Ok(());
        }
        if !self.live.contains_key(key) {
            self.open_bill(OpenBill::new(
                key,
                event.agent_id().clone(),
                event.times().observed_at_ns,
            ))?;
        }
        self.step_live(event)
    }

    /// Absence outcome for `key`.
    ///
    /// A live monitor with no sealed verdict is `Inconclusive`. A retired
    /// monitor returns the outcome stored at retirement, which is
    /// `Inconclusive` when nothing had sealed. Neither case is turned into
    /// `Satisfy`.
    ///
    /// # Errors
    ///
    /// Returns an error when sealing a live monitor fails for a reason other
    /// than an empty evidence gate.
    pub fn absence_outcome(&self, key: &str) -> Result<VerdictOutcome, AdapterError> {
        if let Some(outcome) = self.closed.get(key) {
            return Ok(*outcome);
        }
        let Some(monitor) = self.live.get(key) else {
            return Ok(VerdictOutcome::Inconclusive);
        };
        let Some(watch) = monitor.watch.as_ref() else {
            return Ok(VerdictOutcome::Inconclusive);
        };
        seal_outcome(watch)
    }

    fn history(&self, key: &str) -> &[LedgerEvent] {
        self.live
            .get(key)
            .and_then(|monitor| monitor.watch.as_ref())
            .map(|watch| watch.history.as_slice())
            .unwrap_or(&[])
    }

    fn step_live(&mut self, event: &LedgerEvent) -> Result<(), AdapterError> {
        let Some(key) = event.bill_id().map(|bill| bill.as_str().to_owned()) else {
            return Ok(());
        };
        let Some(monitor) = self.live.get_mut(&key) else {
            return Ok(());
        };
        if monitor.watch.is_none() {
            monitor.watch = Some(Watch {
                evaluator: absence_evaluator(&monitor.agent)?,
                cursor: TraceCursor::new(),
                history: Vec::new(),
            });
        }
        let Some(watch) = monitor.watch.as_mut() else {
            return Ok(());
        };
        let adapted = watch.cursor.push(event)?;
        watch.evaluator.step(Input::event(adapted))?;
        watch.history.push(event.clone());
        Ok(())
    }

    fn preview(&self, key: &str) -> Result<VerdictOutcome, AdapterError> {
        let Some(watch) = self
            .live
            .get(key)
            .and_then(|monitor| monitor.watch.as_ref())
        else {
            return Ok(VerdictOutcome::Inconclusive);
        };
        seal_outcome(watch)
    }

    fn retire(&mut self, key: &str, observed_at_ns: i128) -> Result<(), AdapterError> {
        if !self.is_live(key) {
            return Ok(());
        }
        let outcome = self.preview(key)?;
        self.retire_signal(key, observed_at_ns)?;
        self.live.remove(key);
        self.closed.insert(key.to_owned(), outcome);
        Ok(())
    }

    fn retire_signal(&mut self, key: &str, observed_at_ns: i128) -> Result<(), AdapterError> {
        let horizon = ScopeHorizon::new(ObservationTime::new(sentinel_clock(), observed_at_ns));
        let signal = RetirementSignal::new(Binding::new(key), horizon, RetirementCause::Deadline);
        self.scopes.step(Input::retire(signal))?;
        Ok(())
    }
}

fn kept(key: &str) -> Decision {
    Decision::Kept {
        key: key.to_owned(),
    }
}

fn retired(key: &str) -> Decision {
    Decision::Retired {
        key: key.to_owned(),
    }
}

fn payment_covers(monitor: &mut LiveMonitor, amount: &str, currency: &str) -> bool {
    let Ok(parsed) = amount.trim().parse::<Money>() else {
        return false;
    };
    if parsed < Money::ZERO {
        return false;
    }
    let (Some(owed), Some(expected)) = (monitor.owed, monitor.currency.as_deref()) else {
        return false;
    };
    if currency.trim().is_empty() || !expected.eq_ignore_ascii_case(currency.trim()) {
        return false;
    }
    let Some(next) = monitor.paid.checked_add(parsed) else {
        return false;
    };
    monitor.paid = next;
    next >= owed
}

fn classify(history: &[LedgerEvent], event: LedgerEvent) -> Result<LedgerEvent, AdapterError> {
    if event.kind() != LedgerKind::AgentPayBill {
        return Ok(event);
    }
    let Some(bill) = event.bill_id().map(|bill| bill.as_str().to_owned()) else {
        return Ok(event);
    };
    let already_paid = history.iter().any(|prior| {
        prior.kind() == LedgerKind::AgentPayBill
            && prior.bill_id().is_some_and(|id| id.as_str() == bill)
    });
    if !already_paid {
        return Ok(event);
    }
    let mut extra = event.recast(LedgerKind::ExtraPayment)?;
    if extra.predecessors().is_empty() {
        if let Some(first) = history.iter().find(|prior| {
            prior.kind() == LedgerKind::AgentPayBill
                && prior.bill_id().is_some_and(|id| id.as_str() == bill)
        }) {
            extra.precede_by(first.id())?;
        }
    }
    Ok(extra)
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
        Times::new(
            times.occurred_at_ns.saturating_add(1),
            times.observed_at_ns.saturating_add(1),
        ),
        [extra.id()],
    )?
    .derived())
}

fn seal_outcome(watch: &Watch) -> Result<VerdictOutcome, AdapterError> {
    let evidence = watch.cursor.evidence()?;
    let mut clone = watch.evaluator.clone();
    for source in evidence {
        clone.step(Input::source(source))?;
    }
    let verdicts = match clone.seal() {
        Ok(sealed) => sealed.into_verdicts(),
        Err(CoreError::NoEvidenceCoverage { .. }) => return Ok(VerdictOutcome::Inconclusive),
        Err(error) => return Err(error.into()),
    };
    Ok(strongest(&verdicts))
}

fn strongest(verdicts: &[SentinelVerdict]) -> VerdictOutcome {
    let mut best: Option<VerdictOutcome> = None;
    for verdict in verdicts {
        let outcome = verdict.verdict().outcome();
        best = Some(match best {
            Some(current) if rank(current) >= rank(outcome) => current,
            _ => outcome,
        });
    }
    best.unwrap_or(VerdictOutcome::Inconclusive)
}

fn rank(outcome: VerdictOutcome) -> u8 {
    match outcome {
        VerdictOutcome::Violate => 2,
        VerdictOutcome::Inconclusive => 1,
        VerdictOutcome::Satisfy | _ => 0,
    }
}

impl std::fmt::Debug for MonitorDesk {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let earliest_open_ns = self.live.values().map(|monitor| monitor.opened_at_ns).min();
        formatter
            .debug_struct("MonitorDesk")
            .field("live_count", &self.live.len())
            .field("closed", &self.closed.len())
            .field("earliest_open_ns", &earliest_open_ns)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{BillId, Times};
    use std::str::FromStr;

    fn agent() -> AgentId {
        AgentId::new("payer").expect("agent")
    }

    fn money(amount: &str) -> Money {
        Money::from_str(amount).expect("money")
    }

    fn desk() -> MonitorDesk {
        MonitorDesk::new().expect("desk")
    }

    fn open(desk: &mut MonitorDesk, key: &str, horizon: Option<i128>, owed: Option<&str>) {
        let mut bill = OpenBill::new(key, agent(), 1);
        if let Some(horizon) = horizon {
            bill = bill.with_horizon(horizon);
        }
        if let Some(owed) = owed {
            bill = bill.with_owed(money(owed), "USD");
        }
        desk.open_bill(bill).expect("open");
    }

    fn bill_event(
        id: &str,
        kind: LedgerKind,
        bill: &str,
        at: i128,
        predecessors: &[&str],
    ) -> LedgerEvent {
        LedgerEvent::new(
            id,
            kind,
            BillId::new(bill).expect("bill"),
            agent(),
            Times::at(at),
            predecessors.iter().copied(),
        )
        .expect("event")
    }

    fn push_double_pay(desk: &mut MonitorDesk, bill: &str) {
        let events = [
            bill_event("pay", LedgerKind::AgentPayBill, bill, 10, &[]),
            bill_event("settled", LedgerKind::ObligationSettled, bill, 20, &["pay"]),
            bill_event(
                "pay-again",
                LedgerKind::AgentPayBill,
                bill,
                30,
                &["settled"],
            ),
            bill_event("extra", LedgerKind::ExtraPayment, bill, 40, &["pay-again"]),
            bill_event("credit", LedgerKind::Credit, bill, 50, &["extra"]),
        ];
        for event in events {
            desk.push(event).expect("push");
        }
    }

    #[test]
    fn paid_in_full_retires_and_a_partial_payment_does_not() {
        let mut desk = desk();
        open(&mut desk, "bill-1", None, Some("10.00"));
        let kept = desk
            .record_payment("bill-1", "4.00", "USD", 20)
            .expect("partial");
        assert_eq!(kept, super::kept("bill-1"));
        assert_eq!(desk.live_count(), 1);
        let decision = desk
            .record_payment("bill-1", "6.00", "usd", 30)
            .expect("rest");
        assert_eq!(decision, retired("bill-1"));
        assert_eq!(desk.live_count(), 0);
        assert_eq!(
            desk.absence_outcome("bill-1").expect("outcome"),
            VerdictOutcome::Inconclusive
        );
    }

    #[test]
    fn missing_amount_currency_or_owed_does_not_retire() {
        let mut desk = desk();
        open(&mut desk, "bill-1", None, Some("10.00"));
        assert_eq!(
            desk.record_payment("bill-1", "ten", "USD", 20)
                .expect("bad amount"),
            kept("bill-1")
        );
        assert_eq!(
            desk.record_payment("bill-1", "10.00", "EUR", 20)
                .expect("other currency"),
            kept("bill-1")
        );
        assert_eq!(
            desk.record_payment("bill-1", "-1.00", "USD", 20)
                .expect("negative"),
            kept("bill-1")
        );
        assert!(desk.is_live("bill-1"));

        open(&mut desk, "bill-2", None, None);
        assert_eq!(
            desk.record_payment("bill-2", "10.00", "USD", 20)
                .expect("no owed"),
            kept("bill-2")
        );
        assert!(desk.is_live("bill-2"));
        assert_eq!(desk.live_count(), 2);
    }

    #[test]
    fn horizon_passed_retires_and_a_missing_horizon_does_not() {
        let mut desk = desk();
        open(&mut desk, "due", Some(100), Some("10.00"));
        open(&mut desk, "open", None, Some("10.00"));
        assert_eq!(desk.advance(99).expect("before"), 0);
        assert_eq!(desk.live_count(), 2);
        assert_eq!(desk.advance(100).expect("due"), 1);
        assert!(!desk.is_live("due"));
        assert!(desk.is_live("open"));
        assert_eq!(
            desk.absence_outcome("due").expect("retired"),
            VerdictOutcome::Inconclusive
        );
    }

    #[test]
    fn unknown_close_keeps_the_monitor_and_a_later_double_pay_is_caught() {
        let mut desk = desk();
        open(&mut desk, "bill-1", Some(10_000), Some("500.00"));
        let decision = desk
            .judge_close("bill-1", CloseJudgment::Unknown, 15)
            .expect("unknown");
        assert_eq!(decision, kept("bill-1"));
        assert_eq!(
            desk.judge_close("bill-1", CloseJudgment::Deny, 16)
                .expect("deny"),
            kept("bill-1")
        );
        assert!(desk.is_live("bill-1"));
        push_double_pay(&mut desk, "bill-1");
        assert_eq!(
            desk.absence_outcome("bill-1").expect("absence"),
            VerdictOutcome::Violate
        );
        assert!(!format!("{desk:?}").to_lowercase().contains("fraud"));
    }

    #[test]
    fn an_affirmed_close_retires_and_a_later_payment_is_not_a_pass() {
        let mut desk = desk();
        open(&mut desk, "bill-1", None, Some("500.00"));
        let decision = desk
            .judge_close("bill-1", CloseJudgment::Affirm, 15)
            .expect("affirm");
        assert_eq!(decision, retired("bill-1"));
        push_double_pay(&mut desk, "bill-1");
        assert_eq!(desk.live_count(), 0);
        assert_eq!(
            desk.absence_outcome("bill-1").expect("retired"),
            VerdictOutcome::Inconclusive
        );
    }

    #[test]
    fn live_count_stays_flat_across_a_long_run() {
        let mut desk = desk();
        for index in 0..400 {
            let key = format!("bill-{index}");
            open(&mut desk, &key, Some(index), Some("10.00"));
            if index % 2 == 0 {
                assert_eq!(
                    desk.record_payment(&key, "10.00", "USD", index)
                        .expect("paid"),
                    retired(&key)
                );
            } else {
                assert_eq!(desk.advance(index).expect("horizon"), 1);
            }
            assert!(
                desk.live_count() <= 1,
                "live count grew to {}",
                desk.live_count()
            );
        }
        assert_eq!(desk.live_count(), 0);

        for index in 0..15 {
            open(&mut desk, &format!("held-{index}"), None, Some("10.00"));
            assert_eq!(
                desk.record_payment(&format!("held-{index}"), "1.00", "USD", 1)
                    .expect("partial"),
                kept(&format!("held-{index}"))
            );
        }
        for index in 0..200 {
            let key = format!("gone-{index}");
            open(&mut desk, &key, Some(50), Some("10.00"));
            assert_eq!(
                desk.record_payment(&key, "10.00", "USD", 50).expect("paid"),
                retired(&key)
            );
        }
        desk.advance(50).expect("sweep");
        assert_eq!(desk.live_count(), 15);
    }

    #[test]
    fn alias_unknown_keeps_both_and_affirm_retires_only_the_alias() {
        let mut desk = desk();
        open(&mut desk, "alias", None, Some("10.00"));
        open(&mut desk, "canonical", None, Some("10.00"));
        assert_eq!(
            desk.apply_alias("alias", "canonical", CloseJudgment::Unknown, 5)
                .expect("unknown"),
            kept("alias")
        );
        assert_eq!(desk.live_count(), 2);
        assert_eq!(
            desk.apply_alias("alias", "canonical", CloseJudgment::Affirm, 6)
                .expect("merge"),
            retired("alias")
        );
        assert!(desk.is_live("canonical"));
        assert!(!desk.is_live("alias"));
        assert_eq!(desk.live_count(), 1);
        push_double_pay(&mut desk, "canonical");
        assert_eq!(
            desk.absence_outcome("canonical").expect("still watching"),
            VerdictOutcome::Violate
        );
    }
}
