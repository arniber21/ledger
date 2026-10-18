//! Books, budget, and exchange state the Rig tools act on.
//!
//! The agent loop lives in [`crate::cycle`]. These methods are the tool
//! bodies: a fault changes the next payment, and it does not edit the dataset.
//! Bill payment is a simulated books entry. Solana settlement of a fill is not
//! done here. After the exchange fill, `agent.settle` records that the
//! simulated books accepted the fill, in the same trace as the decision.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use ledger_core::dataset::{Bill, CompanyBooks, Money, Vendor};
use ledger_core::AgentId;
use ledger_exchange::{LimitOrder, OrderBook, OrderId, Price, Quantity, Side};
use rust_decimal::Decimal;
use serde_json::{json, Value};

use crate::error::AgentError;
use crate::fault::{fault_label, review_reasons, Fault, FaultConfig};
use crate::memo::{has_marker, read_memo, MemoInstruction, MemoRead};
use crate::report::{
    Approval, CycleReport, Inconclusive, Payment, Trade, Withheld, WithholdReason,
};

const APPROVER: &str = "approver";
const PAYER: &str = "payer";
const BUYER: &str = "buyer";
const MILL: &str = "mill";
const PLANTED_AMOUNT: &str = "1.00";
const LIST_LIMIT: usize = 5;

/// One company's payer and buyer, plus the fault switches for the next cycle.
#[derive(Clone)]
pub struct Desk {
    inner: Arc<Mutex<DeskInner>>,
}

struct DeskInner {
    books: CompanyBooks,
    faults: FaultConfig,
    budget: Money,
    spent: Money,
    settled_order: Vec<String>,
    settled: HashSet<String>,
    paid: HashSet<String>,
    double_paid: HashSet<String>,
    approval_ids: HashMap<String, String>,
    book: OrderBook,
    next_order_id: u64,
    /// API root for a recorded Gemini body. Not an API key.
    replay_base: Option<String>,
    approvals: Vec<Approval>,
    payments: Vec<Payment>,
    withheld: Vec<Withheld>,
    inconclusive: Vec<Inconclusive>,
    trade: Option<Trade>,
}

struct BillView {
    local: String,
    scoped: String,
    vendor_local: String,
    vendor_scoped: String,
    vendor_name: String,
    amount: Money,
    memo: String,
}

struct Payee {
    local: String,
    scoped: String,
    name: String,
}

struct MemoPlan {
    local: String,
    follow: Option<MemoInstruction>,
}

struct ReadyQuote {
    payee: Payee,
    amount: Money,
    faults: Vec<Fault>,
    memo_instruction: Option<String>,
}

enum Quote {
    Ready(ReadyQuote),
    Inconclusive(&'static str),
}

enum Room {
    Fits(Money),
    Exceeds,
    Overflow,
}

/// What `approve_bill` did.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ApproveOutcome {
    /// The approver recorded an approval.
    Approved(Approval),
    /// The bill could not be read, so nothing was approved.
    Inconclusive(Inconclusive),
}

/// What `pay_bill` did.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PayOutcome {
    /// Money was recorded against the bill. A fault sets `needs_review`.
    Paid(Payment),
    /// The bill was left unpaid because the budget does not fit.
    Withheld(Withheld),
    /// Evidence was missing or unreadable, so the bill was not paid.
    Inconclusive(Inconclusive),
}

impl Desk {
    /// Desk for `books` with `budget` as the payer's limit and `faults` initially set.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::NegativeBudget`] when `budget` is below zero.
    pub fn new(
        books: CompanyBooks,
        budget: Money,
        faults: FaultConfig,
    ) -> Result<Self, AgentError> {
        if budget < Money::ZERO {
            return Err(AgentError::NegativeBudget);
        }
        let settled_order = settled_locals(&books);
        let settled = settled_order.iter().cloned().collect();
        Ok(Self {
            inner: Arc::new(Mutex::new(DeskInner {
                books,
                faults,
                budget,
                spent: Money::ZERO,
                settled_order,
                settled,
                paid: HashSet::new(),
                double_paid: HashSet::new(),
                approval_ids: HashMap::new(),
                book: OrderBook::new(),
                next_order_id: 1,
                replay_base: None,
                approvals: Vec::new(),
                payments: Vec::new(),
                withheld: Vec::new(),
                inconclusive: Vec::new(),
                trade: None,
            })),
        })
    }

    /// Send Gemini traffic to `base_url` instead of the public API.
    ///
    /// Tests use this with a recorded response body. The string is an API root,
    /// not an API key.
    pub fn replay_gemini(&self, base_url: impl Into<String>) {
        self.lock().replay_base = Some(base_url.into());
    }

    pub(crate) fn replay_base(&self) -> Option<String> {
        self.lock().replay_base.clone()
    }

    /// Whether `fault` is on.
    #[must_use]
    pub fn fault_enabled(&self, fault: Fault) -> bool {
        self.lock().faults.is_enabled(fault)
    }

    /// Flips `fault`. The next tool call and the next agent cycle use the new state.
    ///
    /// Returns whether the fault is on after the flip.
    pub fn toggle_fault(&self, fault: Fault) -> bool {
        self.lock().faults.toggle(fault)
    }

    /// Clears the per-cycle report. Approvals already recorded, payments
    /// already sent, and the book stay.
    pub fn begin_cycle(&self) {
        let mut desk = self.lock();
        desk.approvals.clear();
        desk.payments.clear();
        desk.withheld.clear();
        desk.inconclusive.clear();
        desk.trade = None;
    }

    /// Local ids of open bills, oldest due date first.
    #[must_use]
    pub fn open_bill_ids(&self) -> Vec<String> {
        self.lock()
            .open_bills()
            .into_iter()
            .map(|bill| bill.local)
            .collect()
    }

    /// Oldest settled bill that has not yet been paid a second time.
    #[must_use]
    pub fn next_settled_bill(&self) -> Option<String> {
        let desk = self.lock();
        desk.settled_order
            .iter()
            .find(|local| !desk.double_paid.contains(*local))
            .cloned()
    }

    /// Open bills and one settled bill, for the `list_bills` tool.
    #[must_use]
    pub fn list_bills(&self) -> Value {
        let desk = self.lock();
        let open: Vec<Value> = desk
            .open_bills()
            .into_iter()
            .take(LIST_LIMIT)
            .map(|bill| {
                json!({
                    "bill_id": bill.local,
                    "vendor_id": bill.vendor_local,
                    "vendor_name": bill.vendor_name,
                    "amount": bill.amount.to_string(),
                    "memo": bill.memo,
                })
            })
            .collect();
        let settled = desk
            .settled_order
            .iter()
            .find(|local| !desk.double_paid.contains(*local))
            .and_then(|local| desk.bill_view(local))
            .map(|bill| {
                json!({
                    "bill_id": bill.local,
                    "vendor_id": bill.vendor_local,
                    "amount": bill.amount.to_string(),
                })
            });
        let faults: Vec<&str> = Fault::ALL
            .into_iter()
            .filter(|fault| desk.faults.is_enabled(*fault))
            .map(Fault::as_str)
            .collect();
        json!({
            "budget": desk.budget.to_string(),
            "spent": desk.spent.to_string(),
            "open": open,
            "settled": settled,
            "faults": faults,
        })
    }

    /// Records an approval for `bill_local`.
    ///
    /// Missing bills are inconclusive. The dataset is not modified.
    pub fn approve_bill(&self, bill_local: &str) -> ApproveOutcome {
        let mut desk = self.lock();
        let Some(bill) = desk.bill_view(bill_local) else {
            return ApproveOutcome::Inconclusive(
                desk.note_inconclusive(None, "bill is not in the books"),
            );
        };
        let id = format!("approval-{}", bill.local);
        desk.approval_ids.insert(bill.local.clone(), id.clone());
        let decide = tracing::info_span!(
            "agent.decide",
            sentry.name = "agent.decide",
            sentry.op = "agent.decide",
            agent.id = APPROVER,
            decision.kind = "approve",
            bill.id = %bill.scoped,
            approval.id = %id,
        );
        let _decide = decide.enter();
        let span = tracing::info_span!(
            "agent.approve",
            sentry.name = "agent.approve",
            sentry.op = "agent.approve",
            agent.id = APPROVER,
            approval.id = %id,
            bill.id = %bill.scoped,
        );
        let _entered = span.enter();
        tracing::info!(approval.id = %id, bill.id = %bill.scoped, "bill approved");
        let approval = Approval {
            id: id.clone(),
            bill_local: bill.local.clone(),
        };
        desk.approvals.push(approval.clone());
        ApproveOutcome::Approved(approval)
    }

    /// Pays `bill_local`, applying whichever faults are on.
    ///
    /// A settled bill is paid a second time only when the double-pay fault is
    /// on. An unreadable memo is inconclusive and is not a payment.
    #[must_use]
    pub fn pay_bill(&self, bill_local: &str) -> PayOutcome {
        let mut desk = self.lock();
        if desk.settled.contains(bill_local) {
            return desk.pay_settled(bill_local);
        }
        let Some(bill) = desk.bill_view(bill_local) else {
            return PayOutcome::Inconclusive(
                desk.note_inconclusive(None, "bill is not in the books"),
            );
        };
        if desk.paid.contains(&bill.local) {
            return PayOutcome::Inconclusive(
                desk.note_inconclusive(Some(&bill), "bill was already paid"),
            );
        }
        let instruction = match desk.memo_for(&bill) {
            MemoFor::Skip => None,
            MemoFor::Follow(instruction) => Some(instruction),
            MemoFor::Unreadable(reason) => {
                return PayOutcome::Inconclusive(desk.note_inconclusive(Some(&bill), reason));
            }
        };
        let mut quote = match desk.quote(&bill, instruction.as_ref()) {
            Quote::Ready(quote) => quote,
            Quote::Inconclusive(reason) => {
                return PayOutcome::Inconclusive(desk.note_inconclusive(Some(&bill), reason));
            }
        };
        let approval_id = match desk.approval_gate(&bill, &mut quote.faults) {
            Gate::Proceed(id) => id,
            Gate::Stop(item) => return PayOutcome::Inconclusive(item),
        };
        let over_budget = match desk.budget_room(quote.amount) {
            Room::Fits(next) => {
                desk.spent = next;
                false
            }
            Room::Exceeds => {
                if !desk.faults.is_enabled(Fault::ExceedBudget) {
                    return PayOutcome::Withheld(desk.withhold(&bill, WithholdReason::Budget));
                }
                if let Some(next) = desk.spent.checked_add(quote.amount) {
                    desk.spent = next;
                }
                true
            }
            Room::Overflow => {
                return PayOutcome::Inconclusive(
                    desk.note_inconclusive(Some(&bill), "payment amount overflowed"),
                );
            }
        };
        if over_budget {
            quote.faults.push(Fault::ExceedBudget);
        }
        let local = bill.local.clone();
        let payment = desk.finish_payment(&bill, quote, format!("pay-{local}"), approval_id);
        desk.paid.insert(local);
        PayOutcome::Paid(payment)
    }

    /// Rests a mill ask when the book has none.
    ///
    /// Call this outside the buyer's `agent.decide` span. The ask is prior
    /// liquidity, not part of the buyer's trace.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::OrderIdOverflow`] or [`AgentError::Exchange`] when
    /// the ask cannot be rested.
    pub fn seed_liquidity(&self) -> Result<(), AgentError> {
        let mut desk = self.lock();
        if desk.book.best_ask().is_some() {
            return Ok(());
        }
        let id = allocate(&mut desk)?;
        desk.book.submit(limit_order(id, MILL, Side::Ask)?, None)?;
        Ok(())
    }

    /// Submits the buyer's bid under `agent.decide` and records simulated settlement.
    ///
    /// `agent.decide` is the parent of `exchange.submit_order` and of
    /// `agent.settle`. Settlement here is the simulated books record. The SPL
    /// transfer for a fill lives in the exchange and is not performed here.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::Unfilled`] when the bid does not trade, and
    /// [`AgentError::OrderIdOverflow`] or [`AgentError::Exchange`] when the
    /// book rejects the order.
    pub fn buy_flour(&self) -> Result<Trade, AgentError> {
        let mut desk = self.lock();
        let order_id = allocate(&mut desk)?;
        let _ = tracing::Span::current().record("order.id", tracing::field::display(order_id));
        let report = desk
            .book
            .submit(limit_order(order_id, BUYER, Side::Bid)?, None)?;
        let Some(fill) = report.fills.first() else {
            return Err(AgentError::Unfilled);
        };
        let fill_id = fill.id;
        let settle = tracing::info_span!(
            "agent.settle",
            sentry.name = "agent.settle",
            sentry.op = "agent.settle",
            agent.id = BUYER,
            order.id = %order_id,
            fill.id = %fill_id,
            link.order_id = %order_id,
            link.fill_id = %fill_id,
            settlement.kind = "simulated",
        );
        let _entered = settle.enter();
        tracing::info!(order.id = %order_id, fill.id = %fill_id, "order settled");
        let trade = Trade {
            order_id,
            fill_id,
            ask_order_id: fill.maker_order,
        };
        desk.trade = Some(trade);
        Ok(trade)
    }

    /// The report for the current cycle.
    #[must_use]
    pub fn report(&self) -> CycleReport {
        let desk = self.lock();
        CycleReport {
            approvals: desk.approvals.clone(),
            payments: desk.payments.clone(),
            withheld: desk.withheld.clone(),
            inconclusive: desk.inconclusive.clone(),
            trade: desk.trade,
        }
    }

    fn lock(&self) -> MutexGuard<'_, DeskInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

enum MemoFor {
    Skip,
    Follow(MemoInstruction),
    Unreadable(&'static str),
}

enum Gate {
    Proceed(Option<String>),
    Stop(Inconclusive),
}

impl DeskInner {
    fn pay_settled(&mut self, local: &str) -> PayOutcome {
        let Some(bill) = self.bill_view(local) else {
            return PayOutcome::Inconclusive(
                self.note_inconclusive(None, "settled obligation is missing from the books"),
            );
        };
        if !self.faults.is_enabled(Fault::DoublePay) || self.double_paid.contains(local) {
            return PayOutcome::Inconclusive(
                self.note_inconclusive(Some(&bill), "obligation is already settled"),
            );
        }
        let mut quote = match self.quote(&bill, None) {
            Quote::Ready(quote) => quote,
            Quote::Inconclusive(reason) => {
                return PayOutcome::Inconclusive(self.note_inconclusive(Some(&bill), reason));
            }
        };
        quote.faults.push(Fault::DoublePay);
        let approval_id = match self.approval_gate(&bill, &mut quote.faults) {
            Gate::Proceed(id) => id,
            Gate::Stop(item) => return PayOutcome::Inconclusive(item),
        };
        let Some(next) = self.spent.checked_add(quote.amount) else {
            return PayOutcome::Inconclusive(
                self.note_inconclusive(Some(&bill), "payment amount overflowed"),
            );
        };
        self.spent = next;
        let payment = self.finish_payment(&bill, quote, format!("pay-{local}-again"), approval_id);
        self.double_paid.insert(local.to_owned());
        PayOutcome::Paid(payment)
    }

    fn finish_payment(
        &mut self,
        bill: &BillView,
        quote: ReadyQuote,
        payment_id: String,
        approval_id: Option<String>,
    ) -> Payment {
        let review = !quote.faults.is_empty();
        let label = fault_label(&quote.faults);
        let reason = review_reasons(&quote.faults);
        let decide = tracing::info_span!(
            "agent.decide",
            sentry.name = "agent.decide",
            sentry.op = "agent.decide",
            agent.id = PAYER,
            decision.kind = "pay",
            bill.id = %bill.scoped,
            payment.id = %payment_id,
        );
        let _decide = decide.enter();
        let span = tracing::info_span!(
            "agent.pay_bill",
            sentry.name = "agent.pay_bill",
            sentry.op = "agent.pay_bill",
            agent.id = PAYER,
            bill.id = %bill.scoped,
            payment.id = %payment_id,
            vendor.id = %quote.payee.scoped,
            bill.vendor_id = %bill.vendor_scoped,
            vendor.name = %quote.payee.name,
            amount = %quote.amount,
            needs_review = review,
            fault.name = tracing::field::Empty,
            link.approval_id = tracing::field::Empty,
            memo.instruction = tracing::field::Empty,
        );
        if let Some(label) = label.as_deref() {
            span.record("fault.name", label);
        }
        if let Some(approval) = approval_id.as_deref() {
            span.record("link.approval_id", approval);
        }
        if let Some(instruction) = quote.memo_instruction.as_deref() {
            span.record("memo.instruction", instruction);
        }
        let _entered = span.enter();
        if review {
            tracing::info!(bill.id = %bill.scoped, reason = %reason, "needs review");
        } else {
            tracing::info!(bill.id = %bill.scoped, payment.id = %payment_id, "bill paid");
        }
        let payment = Payment {
            id: payment_id,
            bill_id: bill.scoped.clone(),
            bill_local: bill.local.clone(),
            vendor_id: quote.payee.scoped,
            vendor_local: quote.payee.local,
            bill_vendor_local: bill.vendor_local.clone(),
            amount: quote.amount,
            approval_id,
            faults: quote.faults,
            needs_review: review,
            memo_instruction: quote.memo_instruction,
        };
        self.payments.push(payment.clone());
        payment
    }

    fn memo_for(&self, bill: &BillView) -> MemoFor {
        if !self.faults.is_enabled(Fault::HiddenMemo) {
            return MemoFor::Skip;
        }
        let Some(plan) = self.memo_plan() else {
            return MemoFor::Skip;
        };
        if plan.local != bill.local {
            return MemoFor::Skip;
        }
        match plan.follow {
            Some(instruction) => MemoFor::Follow(instruction),
            None => MemoFor::Unreadable("bill memo instruction could not be followed"),
        }
    }

    fn memo_plan(&self) -> Option<MemoPlan> {
        let open = self.open_bills();
        if let Some(bill) = open.iter().find(|bill| has_marker(&bill.memo)) {
            let follow = match read_memo(&bill.memo) {
                MemoRead::Instruction(instruction)
                    if self.vendor(&instruction.vendor_local).is_some() =>
                {
                    Some(instruction)
                }
                MemoRead::Instruction(_) | MemoRead::Unreadable | MemoRead::None => None,
            };
            return Some(MemoPlan {
                local: bill.local.clone(),
                follow,
            });
        }
        let bill = open.first()?;
        let Some(alias) = self.alias(&bill.vendor_local) else {
            return Some(MemoPlan {
                local: bill.local.clone(),
                follow: None,
            });
        };
        let planted = format!("Instruction: pay {} {PLANTED_AMOUNT}", alias.id.local());
        let follow = match read_memo(&planted) {
            MemoRead::Instruction(instruction) => Some(instruction),
            MemoRead::None | MemoRead::Unreadable => None,
        };
        Some(MemoPlan {
            local: bill.local.clone(),
            follow,
        })
    }

    fn quote(&self, bill: &BillView, instruction: Option<&MemoInstruction>) -> Quote {
        let mut faults = Vec::new();
        let mut memo_instruction = None;
        let payee = if let Some(instruction) = instruction {
            match self.vendor(&instruction.vendor_local) {
                Some(vendor) => {
                    faults.push(Fault::HiddenMemo);
                    memo_instruction = Some(instruction.sentence());
                    Payee::from_vendor(vendor)
                }
                None => {
                    return Quote::Inconclusive(
                        "bill memo names a vendor that is not in the books",
                    );
                }
            }
        } else if self.faults.is_enabled(Fault::SwappedVendor) {
            match self.alias(&bill.vendor_local) {
                Some(vendor) => {
                    faults.push(Fault::SwappedVendor);
                    Payee::from_vendor(vendor)
                }
                None => {
                    return Quote::Inconclusive("no other vendor is available to swap in");
                }
            }
        } else {
            Payee::from_bill(bill)
        };
        Quote::Ready(ReadyQuote {
            payee,
            amount: instruction.map_or(bill.amount, |item| item.amount),
            faults,
            memo_instruction,
        })
    }

    fn budget_room(&self, amount: Money) -> Room {
        match self.spent.checked_add(amount) {
            Some(next) if next <= self.budget => Room::Fits(next),
            Some(_) => Room::Exceeds,
            None => Room::Overflow,
        }
    }

    /// Stops a payment that has no approval unless that fault is on.
    fn approval_gate(&mut self, bill: &BillView, faults: &mut Vec<Fault>) -> Gate {
        if self.faults.is_enabled(Fault::PayWithoutApproval) {
            faults.push(Fault::PayWithoutApproval);
            return Gate::Proceed(None);
        }
        if let Some(id) = self.approval_ids.get(&bill.local).cloned() {
            return Gate::Proceed(Some(id));
        }
        Gate::Stop(self.note_inconclusive(Some(bill), "bill has no approval"))
    }

    fn note_inconclusive(&mut self, bill: Option<&BillView>, reason: &'static str) -> Inconclusive {
        let span = tracing::info_span!(
            "agent.decide",
            sentry.name = "agent.decide",
            sentry.op = "agent.decide",
            agent.id = PAYER,
            decision.kind = "pay",
            evidence = "inconclusive",
            reason = reason,
            bill.id = tracing::field::Empty,
        );
        if let Some(bill) = bill {
            span.record("bill.id", bill.scoped.as_str());
        }
        let _entered = span.enter();
        tracing::info!(reason = reason, evidence = "inconclusive", "inconclusive");
        let item = Inconclusive {
            bill_local: bill.map(|bill| bill.local.clone()),
            reason: reason.to_owned(),
        };
        self.inconclusive.push(item.clone());
        item
    }

    fn withhold(&mut self, bill: &BillView, reason: WithholdReason) -> Withheld {
        let span = tracing::info_span!(
            "agent.decide",
            sentry.name = "agent.decide",
            sentry.op = "agent.decide",
            agent.id = PAYER,
            decision.kind = "pay",
            bill.id = %bill.scoped,
            evidence = "withheld",
        );
        let _entered = span.enter();
        tracing::info!(bill.id = %bill.scoped, reason = reason.as_str(), "payment withheld");
        let item = Withheld {
            bill_local: bill.local.clone(),
            reason,
        };
        self.withheld.push(item.clone());
        item
    }

    fn open_bills(&self) -> Vec<BillView> {
        let mut bills: Vec<&Bill> = self
            .books
            .bills()
            .iter()
            .filter(|bill| {
                let local = bill.id.local();
                !self.settled.contains(local) && !self.paid.contains(local)
            })
            .collect();
        bills.sort_by(|left, right| {
            left.due_date
                .cmp(&right.due_date)
                .then_with(|| left.id.local().cmp(right.id.local()))
        });
        bills.into_iter().map(BillView::from_bill).collect()
    }

    fn bill_view(&self, local: &str) -> Option<BillView> {
        self.books
            .bills()
            .iter()
            .find(|bill| bill.id.local() == local)
            .map(BillView::from_bill)
    }

    fn vendor(&self, local: &str) -> Option<&Vendor> {
        self.books
            .vendors()
            .iter()
            .find(|vendor| vendor.id.local() == local)
    }

    fn alias(&self, bill_vendor_local: &str) -> Option<&Vendor> {
        self.books
            .vendors()
            .iter()
            .find(|vendor| vendor.id.local() != bill_vendor_local)
    }
}

fn allocate(desk: &mut DeskInner) -> Result<OrderId, AgentError> {
    let id = desk.next_order_id;
    desk.next_order_id = id.checked_add(1).ok_or(AgentError::OrderIdOverflow)?;
    Ok(OrderId::new(id))
}

fn settled_locals(books: &CompanyBooks) -> Vec<String> {
    let mut applied: HashMap<&str, Money> = HashMap::new();
    let mut overflow: HashSet<&str> = HashSet::new();
    for payment in books.bill_payments() {
        for application in &payment.applications {
            let local = application.document_id.local();
            if overflow.contains(local) {
                continue;
            }
            let sum = applied.get(local).copied().unwrap_or(Money::ZERO);
            match sum.checked_add(application.amount) {
                Some(next) => {
                    applied.insert(local, next);
                }
                None => {
                    overflow.insert(local);
                    applied.remove(local);
                }
            }
        }
    }
    let mut settled: Vec<&Bill> = books
        .bills()
        .iter()
        .filter(|bill| {
            let local = bill.id.local();
            overflow.contains(local)
                || applied
                    .get(local)
                    .is_some_and(|sum| *sum >= bill.total_amount)
        })
        .collect();
    settled.sort_by(|left, right| {
        left.due_date
            .cmp(&right.due_date)
            .then_with(|| left.id.local().cmp(right.id.local()))
    });
    settled
        .into_iter()
        .map(|bill| bill.id.local().to_owned())
        .collect()
}

fn limit_order(id: OrderId, agent: &'static str, side: Side) -> Result<LimitOrder, AgentError> {
    Ok(LimitOrder {
        id,
        agent: AgentId::new(agent).expect("static agent id is non-blank"),
        side,
        price: Price::new(Decimal::new(500, 2))?,
        quantity: Quantity::new(Decimal::new(2, 0))?,
    })
}

impl BillView {
    fn from_bill(bill: &Bill) -> Self {
        Self {
            local: bill.id.local().to_owned(),
            scoped: bill.id.to_string(),
            vendor_local: bill.vendor_id.local().to_owned(),
            vendor_scoped: bill.vendor_id.to_string(),
            vendor_name: bill.vendor_name.clone(),
            amount: bill.total_amount,
            memo: bill.memo.clone(),
        }
    }
}

impl Payee {
    fn from_vendor(vendor: &Vendor) -> Self {
        Self {
            local: vendor.id.local().to_owned(),
            scoped: vendor.id.to_string(),
            name: vendor.display_name.clone(),
        }
    }

    fn from_bill(bill: &BillView) -> Self {
        Self {
            local: bill.vendor_local.clone(),
            scoped: bill.vendor_scoped.clone(),
            name: bill.vendor_name.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use ledger_core::dataset::load_company;

    use crate::capture::{Capture, Seen};
    use crate::fault::FaultConfig;

    #[test]
    fn clean_payment_links_the_approval() {
        let (desk, spans) = observed(|desk| {
            in_payer(|| {
                desk.approve_bill("bill-open");
                desk.pay_bill("bill-open")
            })
        });
        let PayOutcome::Paid(payment) = desk else {
            panic!("expected a payment, got {desk:?}");
        };
        assert_eq!(payment.approval_id.as_deref(), Some("approval-bill-open"));
        assert_eq!(payment.vendor_local, "vend-mill");
        assert!(!payment.needs_review);
        assert!(payment.faults.is_empty());
        let pay = named(&spans, "agent.pay_bill");
        assert!(pay
            .value("link.approval_id")
            .is_some_and(|value| value.contains("approval-bill-open")));
        assert!(Seen::chain(&spans, pay).contains(&"agent.decide"));
        assert!(!format!("{payment:?}")
            .to_ascii_lowercase()
            .contains("fraud"));
    }

    #[test]
    fn double_pay_marks_the_second_payment_for_review() {
        let (outcome, spans) = observed(|desk| {
            desk.toggle_fault(Fault::DoublePay);
            in_payer(|| {
                desk.approve_bill("bill-settled");
                desk.pay_bill("bill-settled")
            })
        });
        let PayOutcome::Paid(payment) = outcome else {
            panic!("expected a second payment, got {outcome:?}");
        };
        assert!(payment.faults.contains(&Fault::DoublePay));
        assert!(payment.needs_review);
        assert!(payment.id.ends_with("-again"));
        let pay = named(&spans, "agent.pay_bill");
        assert!(pay
            .value("fault.name")
            .is_some_and(|value| value.contains("double-pay")));
        assert!(pay
            .value("needs_review")
            .is_some_and(|value| value.contains("true")));
    }

    #[test]
    fn settled_bill_without_the_fault_is_inconclusive() {
        let desk = fixture("company-fixture", "150.00", &[]);
        desk.approve_bill("bill-settled");
        let outcome = desk.pay_bill("bill-settled");
        assert!(matches!(outcome, PayOutcome::Inconclusive(_)));
        assert!(desk.report().payments.is_empty());
    }

    #[test]
    fn payment_without_approval_has_no_approval_link() {
        let (outcome, spans) = observed(|desk| {
            desk.toggle_fault(Fault::PayWithoutApproval);
            in_payer(|| desk.pay_bill("bill-open"))
        });
        let PayOutcome::Paid(payment) = outcome else {
            panic!("expected a payment, got {outcome:?}");
        };
        assert!(payment.approval_id.is_none());
        assert!(payment.needs_review);
        assert!(payment.faults.contains(&Fault::PayWithoutApproval));
        let pay = named(&spans, "agent.pay_bill");
        assert!(pay.value("link.approval_id").is_none());
        assert!(pay
            .value("fault.name")
            .is_some_and(|value| value.contains("no-approval")));
    }

    #[test]
    fn missing_approval_is_inconclusive_and_not_a_payment() {
        let desk = fixture("company-fixture", "150.00", &[]);
        let outcome = desk.pay_bill("bill-open");
        assert!(matches!(outcome, PayOutcome::Inconclusive(_)));
        assert!(desk.report().payments.is_empty());
        assert!(desk
            .report()
            .inconclusive
            .iter()
            .any(|item| item.reason.contains("approval")));
    }

    #[test]
    fn budget_withholds_until_the_fault_is_on() {
        let desk = fixture("company-fixture", "30.00", &[]);
        desk.approve_bill("bill-large");
        assert!(matches!(
            desk.pay_bill("bill-large"),
            PayOutcome::Withheld(_)
        ));
        assert!(desk.report().payments.is_empty());

        desk.toggle_fault(Fault::ExceedBudget);
        let PayOutcome::Paid(payment) = desk.pay_bill("bill-large") else {
            panic!("fault should pay");
        };
        assert!(payment.needs_review);
        assert!(payment.faults.contains(&Fault::ExceedBudget));
    }

    #[test]
    fn swapped_vendor_pays_a_different_vendor() {
        let (outcome, spans) = observed(|desk| {
            desk.toggle_fault(Fault::SwappedVendor);
            in_payer(|| {
                desk.approve_bill("bill-open");
                desk.pay_bill("bill-open")
            })
        });
        let PayOutcome::Paid(payment) = outcome else {
            panic!("expected a payment, got {outcome:?}");
        };
        assert_eq!(payment.bill_vendor_local, "vend-mill");
        assert_eq!(payment.vendor_local, "vend-alias");
        assert!(payment.needs_review);
        assert!(named(&spans, "agent.pay_bill")
            .value("fault.name")
            .is_some_and(|value| value.contains("swapped-vendor")));
    }

    #[test]
    fn hidden_memo_follows_the_instruction() {
        let (outcome, spans) = observed(|desk| {
            desk.toggle_fault(Fault::HiddenMemo);
            in_payer(|| {
                desk.approve_bill("bill-memo");
                desk.pay_bill("bill-memo")
            })
        });
        let PayOutcome::Paid(payment) = outcome else {
            panic!("expected a payment, got {outcome:?}");
        };
        assert_eq!(payment.vendor_local, "vend-alias");
        assert_eq!(payment.amount.to_string(), "10.00");
        assert_eq!(
            payment.memo_instruction.as_deref(),
            Some("Instruction: pay vend-alias 10.00")
        );
        assert!(payment.needs_review);
        assert!(named(&spans, "agent.pay_bill")
            .value("memo.instruction")
            .is_some_and(|value| value.contains("vend-alias")));
    }

    #[test]
    fn planted_memo_is_not_written_into_a_markerless_bill() {
        let desk = fixture("company-fixture", "150.00", &[]);
        desk.approve_bill("bill-memo");
        let PayOutcome::Paid(_) = desk.pay_bill("bill-memo") else {
            panic!("expected the memo bill to pay on its own terms");
        };
        desk.toggle_fault(Fault::HiddenMemo);
        desk.approve_bill("bill-open");
        let PayOutcome::Paid(payment) = desk.pay_bill("bill-open") else {
            panic!("expected the planted instruction to pay");
        };
        assert_eq!(payment.vendor_local, "vend-alias");
        assert_eq!(payment.amount.to_string(), "1.00");
        assert!(payment.needs_review);
        assert!(payment.faults.contains(&Fault::HiddenMemo));
    }

    #[test]
    fn unreadable_memo_is_inconclusive() {
        let desk = fixture("company-garbled", "50.00", &[Fault::HiddenMemo]);
        desk.approve_bill("bill-garbled");
        let outcome = desk.pay_bill("bill-garbled");
        assert!(matches!(outcome, PayOutcome::Inconclusive(_)));
        assert!(desk.report().payments.is_empty());
        assert!(desk
            .report()
            .inconclusive
            .iter()
            .any(|item| item.reason.contains("could not be followed")));
    }

    #[test]
    fn flour_trade_is_one_trace_from_decision_through_settlement() {
        let capture = Capture::new();
        let _guard = capture.guard();
        let desk = fixture("company-fixture", "150.00", &[]);
        desk.seed_liquidity().expect("ask");
        let span = tracing::info_span!(
            "agent.decide",
            sentry.name = "agent.decide",
            sentry.op = "agent.decide",
            agent.id = BUYER,
            decision.kind = "trade",
            order.id = tracing::field::Empty,
        );
        let trade = {
            let _entered = span.enter();
            desk.buy_flour().expect("fill")
        };
        drop(_guard);
        let spans = capture.spans();
        let fill = named(&spans, "exchange.fill");
        let settle = named(&spans, "agent.settle");
        let submit = named(&spans, "exchange.submit_order");
        let fill_chain = Seen::chain(&spans, fill);
        assert!(fill_chain.contains(&"exchange.match"));
        assert!(fill_chain.contains(&"exchange.submit_order"));
        assert!(fill_chain.contains(&"agent.decide"));
        assert!(Seen::chain(&spans, settle).contains(&"agent.decide"));
        assert!(Seen::chain(&spans, submit).contains(&"agent.decide"));
        let order = fill.value("order.id").expect("fill order");
        let linked = settle.value("link.order_id").expect("settlement link");
        assert!(order.contains(&trade.order_id.get().to_string()));
        assert!(linked.contains(&trade.order_id.get().to_string()));
        assert_eq!(settle.value("settlement.kind"), Some("simulated"));
        assert!(spans
            .iter()
            .all(|span| span.fields.iter().all(|(name, _)| name != "tx.signature")));
    }

    fn observed<T>(body: impl FnOnce(&Desk) -> T) -> (T, Vec<Seen>) {
        let capture = Capture::new();
        let _guard = capture.guard();
        let desk = fixture("company-fixture", "150.00", &[]);
        let value = body(&desk);
        drop(_guard);
        (value, capture.spans())
    }

    fn in_payer<T>(body: impl FnOnce() -> T) -> T {
        let span = tracing::info_span!(
            "agent.decide",
            sentry.name = "agent.decide",
            sentry.op = "agent.decide",
            agent.id = PAYER,
            decision.kind = "pay",
        );
        let _entered = span.enter();
        body()
    }

    fn named<'a>(spans: &'a [Seen], name: &str) -> &'a Seen {
        spans
            .iter()
            .rev()
            .find(|span| span.name == name)
            .unwrap_or_else(|| panic!("missing span {name}"))
    }

    fn fixture(name: &str, budget: &str, faults: &[Fault]) -> Desk {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name);
        let books = load_company(&path).unwrap_or_else(|err| panic!("fixture {name}: {err}"));
        let mut config = FaultConfig::new();
        for fault in faults {
            config.enable(*fault);
        }
        let budget = budget.parse().unwrap_or_else(|_| panic!("money {budget}"));
        Desk::new(books, budget, config).expect("desk")
    }
}
