//! Simulated buying agents for one company.
//!
//! Three scripted agents share a desk:
//! - `approver` records an approval before a bill is paid
//! - `payer` pays that company's open bills inside a budget
//! - `buyer` lifts a resting flour ask on the exchange
//!
//! Five faults are toggled by name and change the next cycle. They are not
//! edits to the dataset. A fault payment is "needs review". A memo instruction
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
mod desk;
mod error;
mod fault;
mod memo;
mod report;

pub use control::{listen, toggle_running, CONTROL_SOCKET};
pub use desk::Desk;
pub use error::AgentError;
pub use fault::{Fault, FaultConfig};
pub use report::{Approval, CycleReport, Inconclusive, Payment, Trade, Withheld, WithholdReason};
