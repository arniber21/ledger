//! Rig tools the approver, payer, and buyer call.
//!
//! [`rig_core::tool::Tool::call`] runs only when Rig's agent loop asks for the
//! tool. The desk methods are the work; this module is the schema and the
//! adapter.

use std::future::Future;

use rig_core::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::Value;

use crate::desk::{ApproveOutcome, Desk, PayOutcome};
use crate::error::AgentError;

const BILL_PARAMETERS: &str = r#"{
  "type": "object",
  "properties": {
    "bill_id": {
      "type": "string",
      "description": "Local bill id from the company books, such as bill-open."
    }
  },
  "required": ["bill_id"]
}"#;

const EMPTY_PARAMETERS: &str = r#"{
  "type": "object",
  "properties": {},
  "additionalProperties": true
}"#;

#[derive(Clone)]
pub(crate) struct ListBills {
    pub(crate) desk: Desk,
}

#[derive(Clone)]
pub(crate) struct ApproveBill {
    pub(crate) desk: Desk,
}

#[derive(Clone)]
pub(crate) struct PayBill {
    pub(crate) desk: Desk,
}

#[derive(Clone)]
pub(crate) struct BuyFlour {
    pub(crate) desk: Desk,
}

#[derive(Debug, Deserialize)]
pub(crate) struct BillArgs {
    #[serde(alias = "billId")]
    bill_id: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct EmptyArgs {}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ToolReply {
    status: &'static str,
    detail: String,
}

impl Tool for ListBills {
    const NAME: &'static str = "list_bills";
    type Args = EmptyArgs;
    type Output = Value;
    type Error = AgentError;

    fn description(&self) -> String {
        "List up to five open bills and one settled bill, oldest due date first, plus the budget and the faults that are on.".to_owned()
    }

    fn parameters(&self) -> Value {
        parse_schema(EMPTY_PARAMETERS)
    }

    fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send {
        let desk = self.desk.clone();
        async move { Ok(desk.list_bills()) }
    }
}

impl Tool for ApproveBill {
    const NAME: &'static str = "approve_bill";
    type Args = BillArgs;
    type Output = ToolReply;
    type Error = AgentError;

    fn description(&self) -> String {
        "Record an approval for a bill. Call this before pay_bill, unless the no-approval fault is on.".to_owned()
    }

    fn parameters(&self) -> Value {
        parse_schema(BILL_PARAMETERS)
    }

    fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send {
        let desk = self.desk.clone();
        async move { Ok(reply_approval(desk.approve_bill(&args.bill_id))) }
    }
}

impl Tool for PayBill {
    const NAME: &'static str = "pay_bill";
    type Args = BillArgs;
    type Output = ToolReply;
    type Error = AgentError;

    fn description(&self) -> String {
        "Pay one bill. A fault that changes the payment returns status \"needs review\". An unreadable memo returns status \"inconclusive\" and does not pay.".to_owned()
    }

    fn parameters(&self) -> Value {
        parse_schema(BILL_PARAMETERS)
    }

    fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send {
        let desk = self.desk.clone();
        async move { Ok(reply_payment(desk.pay_bill(&args.bill_id))) }
    }
}

impl Tool for BuyFlour {
    const NAME: &'static str = "buy_flour";
    type Args = EmptyArgs;
    type Output = ToolReply;
    type Error = AgentError;

    fn description(&self) -> String {
        "Buy flour by lifting the resting ask on the exchange. Call this once.".to_owned()
    }

    fn parameters(&self) -> Value {
        parse_schema(EMPTY_PARAMETERS)
    }

    fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send {
        let desk = self.desk.clone();
        async move {
            let trade = desk.buy_flour()?;
            Ok(ToolReply {
                status: "filled",
                detail: format!("order {} fill {}", trade.order_id, trade.fill_id),
            })
        }
    }
}

fn reply_approval(outcome: ApproveOutcome) -> ToolReply {
    match outcome {
        ApproveOutcome::Approved(approval) => ToolReply {
            status: "approved",
            detail: format!("approval {} for bill {}", approval.id, approval.bill_local),
        },
        ApproveOutcome::Inconclusive(item) => ToolReply {
            status: "inconclusive",
            detail: item.reason,
        },
    }
}

fn reply_payment(outcome: PayOutcome) -> ToolReply {
    match outcome {
        PayOutcome::Paid(payment) if payment.needs_review => ToolReply {
            status: "needs review",
            detail: format!(
                "bill {} amount {} vendor {}",
                payment.bill_local, payment.amount, payment.vendor_local
            ),
        },
        PayOutcome::Paid(payment) => ToolReply {
            status: "paid",
            detail: format!(
                "bill {} amount {} vendor {}",
                payment.bill_local, payment.amount, payment.vendor_local
            ),
        },
        PayOutcome::Withheld(withheld) => ToolReply {
            status: "withheld",
            detail: format!(
                "bill {} reason {}",
                withheld.bill_local,
                withheld.reason.as_str()
            ),
        },
        PayOutcome::Inconclusive(item) => ToolReply {
            status: "inconclusive",
            detail: item.reason,
        },
    }
}

fn parse_schema(schema: &str) -> Value {
    serde_json::from_str(schema).expect("tool schema is json")
}
