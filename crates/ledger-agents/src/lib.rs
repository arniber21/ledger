//! Simulated buying agents for one company.
//!
//! Three Rig agents share a desk. Rig (`rig-core` Gemini provider and tools,
//! `rig-agent` builder) runs the tool loop. This crate does not dispatch tools
//! itself.
//! - `approver` calls `approve_bill` before a bill is paid
//! - `payer` calls `pay_bill` for that company's bills, inside a budget
//! - `buyer` calls `buy_flour` and lifts a resting flour ask on the exchange
//!
//! Five faults are toggled by name and change what those tools do. They are
//! not edits to the dataset. A fault payment is "needs review". Jev answers
//! whether a memo instructs a payment. A yes, at the fixed threshold, flags
//! that payment. A no does not. A timeout, error, contradiction, or score
//! under the threshold is unknown: the bill is left unpaid. A memo instruction
//! that cannot be read is inconclusive and is not paid as a normal bill.
//!
//! Trace shape for a trade, one tree under `agent.decide`:
//! `agent.decide` → `exchange.submit_order` → `exchange.match` → `exchange.fill`,
//! and `agent.settle` in that same tree. `agent.settle` is the simulated books
//! record after the fill. It is not a Solana transfer and has no transaction
//! signature.
//!
//! Approval and payment belong to different agents. The payment span carries
//! `link.approval_id`. The fill span carries `order.id`, and `agent.settle`
//! carries `link.order_id` for the same order.

mod control;
mod cycle;
mod desk;
mod error;
mod fault;
mod memo;
mod predicate;
mod report;
mod tools;

#[cfg(test)]
mod capture;
#[cfg(test)]
mod replay;

pub use control::{listen, toggle_running, CONTROL_SOCKET};
pub use cycle::GEMINI_MODEL;
pub use desk::{ApproveOutcome, Desk, PayOutcome};
pub use error::AgentError;
pub use fault::{Fault, FaultConfig};
pub use report::{Approval, CycleReport, Inconclusive, Payment, Trade, Withheld, WithholdReason};
