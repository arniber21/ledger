//! Scripted approver, payer, and buyer for one company's books.
//!
//! Decisions are deterministic. A fault changes the next decision; it does not
//! edit the dataset. Bill payment is a simulated books entry. The buyer trades
//! flour on the exchange already on main. Solana settlement of a fill is not
//! done here: after the exchange fill, `agent.settle` records that the
//! simulated books accepted the fill, in the same trace as the decision.

use std::collections::{HashMap, HashSet};

use ledger_core::dataset::{Bill, CompanyBooks, Money, Vendor};
use ledger_core::AgentId;
use ledger_exchange::{LimitOrder, OrderBook, OrderId, Price, Quantity, Side};
use rust_decimal::Decimal;

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

/// One company's payer and buyer, plus the fault switches for the next cycle.
pub struct Desk {
    books: CompanyBooks,
    faults: FaultConfig,
    budget: Money,
    spent: Money,
    /// Local ids settled in the loaded books, earliest due date first.
    settled_order: Vec<String>,
    settled: HashSet<String>,
    paid: HashSet<String>,
    double_paid: HashSet<String>,
    book: OrderBook,
    next_order_id: u64,
}

/// A bill reduced to the fields the payer needs, owned so the books stay borrowed.
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
    /// `None` means the marker or the named vendor could not be used.
    follow: Option<MemoInstruction>,
}

struct PayPlan {
    payment_id: String,
    bill_local: String,
    bill_scoped: String,
    bill_vendor_local: String,
    bill_vendor_scoped: String,
    payee_local: String,
    payee_scoped: String,
    payee_name: String,
    amount: Money,
    approval_id: Option<String>,
    faults: Vec<Fault>,
    memo_instruction: Option<String>,
}

struct PartialReport {
    approvals: Vec<Approval>,
    payments: Vec<Payment>,
    withheld: Vec<Withheld>,
    inconclusive: Vec<Inconclusive>,
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
            books,
            faults,
            budget,
            spent: Money::ZERO,
            settled_order,
            settled,
            paid: HashSet::new(),
            double_paid: HashSet::new(),
            book: OrderBook::new(),
            next_order_id: 1,
        })
    }

    /// Whether `fault` is on.
    #[must_use]
    pub fn fault_enabled(&self, fault: Fault) -> bool {
        self.faults.is_enabled(fault)
    }

    /// Flips `fault`. The next [`Desk::run_cycle`] uses the new state.
    ///
    /// Returns whether the fault is on after the flip.
    #[must_use]
    pub fn toggle_fault(&mut self, fault: Fault) -> bool {
        self.faults.toggle(fault)
    }

    /// Pays open bills, maybe a settled one, then buys flour.
    ///
    /// The buyer's `agent.decide` span is the parent of `exchange.submit_order`,
    /// `exchange.match`, and `exchange.fill`. `agent.settle` is the simulated
    /// settlement in that same trace. It is not an SPL transfer and carries no
    /// transaction signature.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::Unfilled`] when the buyer's order does not trade,
    /// [`AgentError::OrderIdOverflow`] when the order counter overflows, and
    /// [`AgentError::Exchange`] when the book rejects an order.
    #[must_use = "the report lists payments, withholds, and the trade"]
    pub fn run_cycle(&mut self) -> Result<CycleReport, AgentError> {
        let mut partial = PartialReport {
            approvals: Vec::new(),
            payments: Vec::new(),
            withheld: Vec::new(),
            inconclusive: Vec::new(),
        };
        self.pay(&mut partial);
        Ok(CycleReport {
            approvals: partial.approvals,
            payments: partial.payments,
            withheld: partial.withheld,
            inconclusive: partial.inconclusive,
            trade: self.trade()?,
        })
    }

    fn pay(&mut self, report: &mut PartialReport) {
        let open = self.open_bills();
        let memo = self.memo_plan(&open);
        if self.faults.is_enabled(Fault::HiddenMemo) && open.is_empty() {
            self.note_inconclusive(report, None, "no open bill carries a memo instruction");
        }

        let mut stop = false;
        for bill in &open {
            if stop {
                break;
            }
            if let Some(plan) = memo.as_ref().filter(|plan| plan.local == bill.local) {
                if plan.follow.is_none() {
                    self.note_inconclusive(
                        report,
                        Some(bill),
                        "bill memo instruction could not be followed",
                    );
                    continue;
                }
            }
            let instruction = memo.as_ref().and_then(|plan| {
                if plan.local == bill.local {
                    plan.follow.clone()
                } else {
                    None
                }
            });
            match self.quote(bill, instruction.as_ref()) {
                Quote::Inconclusive(reason) => {
                    self.note_inconclusive(report, Some(bill), reason);
                }
                Quote::Ready(quote) => match self.budget_room(quote.amount) {
                    Room::Overflow => {
                        self.note_inconclusive(report, Some(bill), "payment amount overflowed");
                        stop = true;
                    }
                    Room::Exceeds => {
                        if self.faults.is_enabled(Fault::ExceedBudget) {
                            self.pay_quote(report, bill, quote, true);
                        } else {
                            withhold(report, bill, WithholdReason::Budget);
                        }
                        stop = true;
                    }
                    Room::Fits(next) => {
                        self.pay_quote(report, bill, quote, false);
                        self.spent = next;
                    }
                },
            }
        }

        if self.faults.is_enabled(Fault::DoublePay) {
            self.pay_settled_again(report);
        }
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

    fn pay_quote(
        &mut self,
        report: &mut PartialReport,
        bill: &BillView,
        mut quote: ReadyQuote,
        over_budget: bool,
    ) {
        if over_budget {
            quote.faults.push(Fault::ExceedBudget);
            if let Some(next) = self.spent.checked_add(quote.amount) {
                self.spent = next;
            }
        }
        let approval_id = self.approval_for(report, bill, &mut quote.faults);
        let plan = PayPlan {
            payment_id: format!("pay-{}", bill.local),
            bill_local: bill.local.clone(),
            bill_scoped: bill.scoped.clone(),
            bill_vendor_local: bill.vendor_local.clone(),
            bill_vendor_scoped: bill.vendor_scoped.clone(),
            payee_local: quote.payee.local,
            payee_scoped: quote.payee.scoped,
            payee_name: quote.payee.name,
            amount: quote.amount,
            approval_id,
            faults: quote.faults,
            memo_instruction: quote.memo_instruction,
        };
        self.emit_payment(report, &plan);
        self.paid.insert(bill.local.clone());
    }

    fn pay_settled_again(&mut self, report: &mut PartialReport) {
        let Some(local) = self
            .settled_order
            .iter()
            .find(|local| !self.double_paid.contains(*local))
            .cloned()
        else {
            tracing::info!("settled obligations already have a second payment");
            return;
        };
        let Some(bill) = self.bill_view(&local) else {
            self.note_inconclusive(report, None, "settled obligation is missing from the books");
            return;
        };
        let quote = match self.quote(&bill, None) {
            Quote::Ready(quote) => quote,
            Quote::Inconclusive(reason) => {
                self.note_inconclusive(report, Some(&bill), reason);
                return;
            }
        };
        if let Some(next) = self.spent.checked_add(quote.amount) {
            self.spent = next;
        } else {
            self.note_inconclusive(report, Some(&bill), "payment amount overflowed");
            return;
        }
        let mut faults = quote.faults;
        faults.push(Fault::DoublePay);
        let approval_id = self.approval_for(report, &bill, &mut faults);
        let plan = PayPlan {
            payment_id: format!("pay-{local}-again"),
            bill_local: bill.local.clone(),
            bill_scoped: bill.scoped,
            bill_vendor_local: bill.vendor_local,
            bill_vendor_scoped: bill.vendor_scoped,
            payee_local: quote.payee.local,
            payee_scoped: quote.payee.scoped,
            payee_name: quote.payee.name,
            amount: quote.amount,
            approval_id,
            faults,
            memo_instruction: quote.memo_instruction,
        };
        self.emit_payment(report, &plan);
        self.double_paid.insert(local);
    }

    fn approval_for(
        &self,
        report: &mut PartialReport,
        bill: &BillView,
        faults: &mut Vec<Fault>,
    ) -> Option<String> {
        if self.faults.is_enabled(Fault::PayWithoutApproval) {
            faults.push(Fault::PayWithoutApproval);
            return None;
        }
        let id = format!("approval-{}", bill.local);
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
        let approve = tracing::info_span!(
            "agent.approve",
            sentry.name = "agent.approve",
            sentry.op = "agent.approve",
            agent.id = APPROVER,
            approval.id = %id,
            bill.id = %bill.scoped,
        );
        let _approve = approve.enter();
        tracing::info!(approval.id = %id, bill.id = %bill.scoped, "bill approved");
        report.approvals.push(Approval {
            id: id.clone(),
            bill_local: bill.local.clone(),
        });
        Some(id)
    }

    fn emit_payment(&self, report: &mut PartialReport, plan: &PayPlan) {
        let review = !plan.faults.is_empty();
        let label = fault_label(&plan.faults);
        let reason = review_reasons(&plan.faults);
        let decide = tracing::info_span!(
            "agent.decide",
            sentry.name = "agent.decide",
            sentry.op = "agent.decide",
            agent.id = PAYER,
            decision.kind = "pay",
            bill.id = %plan.bill_scoped,
            payment.id = %plan.payment_id,
        );
        let _decide = decide.enter();
        let pay = tracing::info_span!(
            "agent.pay_bill",
            sentry.name = "agent.pay_bill",
            sentry.op = "agent.pay_bill",
            agent.id = PAYER,
            bill.id = %plan.bill_scoped,
            payment.id = %plan.payment_id,
            vendor.id = %plan.payee_scoped,
            bill.vendor_id = %plan.bill_vendor_scoped,
            vendor.name = %plan.payee_name,
            amount = %plan.amount,
            needs_review = review,
            fault.name = tracing::field::Empty,
            link.approval_id = tracing::field::Empty,
            memo.instruction = tracing::field::Empty,
        );
        if let Some(label) = label.as_deref() {
            pay.record("fault.name", label);
        }
        if let Some(approval) = plan.approval_id.as_deref() {
            pay.record("link.approval_id", approval);
        }
        if let Some(instruction) = plan.memo_instruction.as_deref() {
            pay.record("memo.instruction", instruction);
        }
        let _pay = pay.enter();
        if review {
            tracing::info!(bill.id = %plan.bill_scoped, reason = %reason, "needs review");
        } else {
            tracing::info!(
                bill.id = %plan.bill_scoped,
                payment.id = %plan.payment_id,
                "bill paid"
            );
        }
        report.payments.push(Payment {
            id: plan.payment_id.clone(),
            bill_id: plan.bill_scoped.clone(),
            bill_local: plan.bill_local.clone(),
            vendor_id: plan.payee_scoped.clone(),
            vendor_local: plan.payee_local.clone(),
            bill_vendor_local: plan.bill_vendor_local.clone(),
            amount: plan.amount,
            approval_id: plan.approval_id.clone(),
            faults: plan.faults.clone(),
            needs_review: review,
            memo_instruction: plan.memo_instruction.clone(),
        });
    }

    fn note_inconclusive(&self, report: &mut PartialReport, bill: Option<&BillView>, reason: &str) {
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
        report.inconclusive.push(Inconclusive {
            bill_local: bill.map(|bill| bill.local.clone()),
            reason: reason.to_owned(),
        });
    }

    /// Rests a mill ask, then buys it inside `agent.decide`.
    ///
    /// The ask is prior liquidity, outside the buyer's trace. The bid is
    /// submitted with that trace entered, so the exchange spans continue it.
    /// `sentry-trace` is left unset: the active span is the parent, which is
    /// how [`OrderBook::submit`](ledger_exchange::OrderBook::submit) continues
    /// an in-process caller.
    fn trade(&mut self) -> Result<Trade, AgentError> {
        let ask_order_id = self.allocate_order_id()?;
        self.book
            .submit(limit_order(ask_order_id, MILL, Side::Ask)?, None)?;

        let order_id = self.allocate_order_id()?;
        let decide = tracing::info_span!(
            "agent.decide",
            sentry.name = "agent.decide",
            sentry.op = "agent.decide",
            agent.id = BUYER,
            decision.kind = "trade",
            order.id = %order_id,
        );
        let _decide = decide.enter();
        let report = self
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
        let _settle = settle.enter();
        tracing::info!(order.id = %order_id, fill.id = %fill_id, "order settled");
        Ok(Trade {
            order_id,
            fill_id,
            ask_order_id,
        })
    }

    fn allocate_order_id(&mut self) -> Result<OrderId, AgentError> {
        let id = self.next_order_id;
        self.next_order_id = id.checked_add(1).ok_or(AgentError::OrderIdOverflow)?;
        Ok(OrderId::new(id))
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

    fn memo_plan(&self, open: &[BillView]) -> Option<MemoPlan> {
        if !self.faults.is_enabled(Fault::HiddenMemo) {
            return None;
        }
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

    fn vendor(&self, local: &str) -> Option<&Vendor> {
        self.books
            .vendors()
            .iter()
            .find(|vendor| vendor.id.local() == local)
    }

    /// First vendor in file order that is not `bill_vendor_local`.
    fn alias(&self, bill_vendor_local: &str) -> Option<&Vendor> {
        self.books
            .vendors()
            .iter()
            .find(|vendor| vendor.id.local() != bill_vendor_local)
    }
}

enum Quote {
    Ready(ReadyQuote),
    Inconclusive(&'static str),
}

struct ReadyQuote {
    payee: Payee,
    amount: Money,
    faults: Vec<Fault>,
    memo_instruction: Option<String>,
}

enum Room {
    Fits(Money),
    Exceeds,
    Overflow,
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

fn withhold(report: &mut PartialReport, bill: &BillView, reason: WithholdReason) {
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
    report.withheld.push(Withheld {
        bill_local: bill.local.clone(),
        reason,
    });
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
