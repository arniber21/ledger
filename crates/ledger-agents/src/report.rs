//! What one decision cycle did: approvals, payments, withholds, and a trade.

use ledger_core::dataset::Money;
use ledger_exchange::{FillId, OrderId};

use crate::fault::Fault;

/// One cycle of the approver, the payer, and the buyer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CycleReport {
    /// Approvals the approver recorded before a payment.
    pub approvals: Vec<Approval>,
    /// Payments the payer sent.
    pub payments: Vec<Payment>,
    /// Open bills the payer refused, with the reason.
    pub withheld: Vec<Withheld>,
    /// Decisions that stopped because the evidence could not be read.
    pub inconclusive: Vec<Inconclusive>,
    /// The buyer's fill against resting flour liquidity.
    pub trade: Trade,
}

/// An approval that must precede a payment, unless the no-approval fault is on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Approval {
    /// Stable id shared with the payment's `link.approval_id` span field.
    pub id: String,
    /// Local bill id from the company books.
    pub bill_local: String,
}

/// A simulated bill payment. Not an SPL transfer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Payment {
    /// Id of this payment decision.
    pub id: String,
    /// Company-scoped bill id.
    pub bill_id: String,
    /// Local bill id from the company books.
    pub bill_local: String,
    /// Company-scoped vendor the money was sent to.
    pub vendor_id: String,
    /// Local id of the vendor the money was sent to.
    pub vendor_local: String,
    /// Local id of the vendor named on the bill.
    pub bill_vendor_local: String,
    /// Amount sent.
    pub amount: Money,
    /// Approval this payment links to. Absent when the bill was not approved.
    pub approval_id: Option<String>,
    /// Faults that changed this payment. Empty when the payer followed the bill.
    pub faults: Vec<Fault>,
    /// True when a fault changed the payment. The payer says "needs review".
    pub needs_review: bool,
    /// Instruction sentence the payer followed, when the hidden-memo fault applied.
    pub memo_instruction: Option<String>,
}

/// A bill the payer did not pay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Withheld {
    /// Local bill id.
    pub bill_local: String,
    /// Why the payer stopped.
    pub reason: WithholdReason,
}

/// Why a bill was not paid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WithholdReason {
    /// The bill's amount does not fit the remaining budget.
    Budget,
}

impl WithholdReason {
    /// Short reason recorded on the withhold event.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Budget => "budget",
        }
    }
}

/// A decision with missing or unreadable evidence. Not a payment and not a pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Inconclusive {
    /// Local bill id, when a bill was in hand.
    pub bill_local: Option<String>,
    /// What could not be read.
    pub reason: String,
}

/// One filled flour order. Settlement here is the simulated books record.
///
/// The Solana transfer for this fill is a separate step and is not performed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Trade {
    /// The buyer's order.
    pub order_id: OrderId,
    /// The fill that order took.
    pub fill_id: FillId,
    /// The resting ask the buyer lifted.
    pub ask_order_id: OrderId,
}
