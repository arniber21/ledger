//! One decision cycle: approver, then payer, then buyer.
//!
//! Each role is a Rig agent. [`rig_agent::AgentBuilder`] attaches the desk
//! tools and [`AgentRunner`](rig_agent::AgentRunner) executes the tool calls.
//! Gemini is the only model provider. A replay base URL, used by tests, serves
//! a recorded Gemini body through that same client.

use rig_agent::AgentBuilder;
use rig_core::providers::gemini::completion::{GenerateContent, GEMINI_3_8_FLASH};
use rig_core::providers::gemini::{Gemini, GeminiConfig};
use rig_core::Model;
use tracing::Instrument;

use crate::desk::Desk;
use crate::error::AgentError;
use crate::fault::Fault;
use crate::report::CycleReport;
use crate::tools::{ApproveBill, BuyFlour, ListBills, PayBill};

/// Gemini model the buying agents call. Same id ledger-spec uses for drafts.
pub const GEMINI_MODEL: &str = GEMINI_3_8_FLASH;

const APPROVER_PREAMBLE: &str = "\
You approve bills for one company before they are paid. Call list_bills, then \
approve_bill once for each open bill id it returns. Do not pay bills.";

const APPROVER_PROMPT: &str = "Approve each open bill from list_bills.";

const PAYER_PREAMBLE: &str = "\
You pay this company's bills. Call list_bills, then pay_bill for each open bill \
id that fits the budget. When a tool returns needs review, leave that wording \
as the outcome. Never say fraud. If a tool returns inconclusive, leave that \
bill unpaid.";

const BUYER_PREAMBLE: &str = "\
You buy flour for the company. Call buy_flour once to lift the resting ask.";

const BUYER_PROMPT: &str = "Buy flour.";

impl Desk {
    /// Runs the approver, the payer, and the buyer through Rig.
    ///
    /// The buyer's `agent.decide` span is the parent of the exchange spans and
    /// of `agent.settle`. Liquidity is seeded before that span.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::MissingGeminiKey`] when no replay URL is set and
    /// `GEMINI_API_KEY` is missing. Returns [`AgentError::Gemini`] when the
    /// provider rejects the run. The error text does not contain the key.
    /// Returns [`AgentError::Unfilled`] when `buy_flour` is called and does
    /// not trade.
    pub async fn run_cycle(&self) -> Result<CycleReport, AgentError> {
        self.begin_cycle();
        if !self.fault_enabled(Fault::PayWithoutApproval) {
            let model = self.completion_model()?;
            run_approver(self, model).await?;
        }
        let model = self.completion_model()?;
        run_payer(self, model).await?;
        self.seed_liquidity()?;
        let model = self.completion_model()?;
        run_buyer(self, model).await?;
        if self.report().trade.is_none() {
            tracing::warn!("buyer did not submit an order");
        }
        Ok(self.report())
    }

    fn completion_model(&self) -> Result<Model<GenerateContent>, AgentError> {
        if let Some(base) = self.replay_base() {
            let http = rig_reqwest::reqwest::Client::builder()
                .no_proxy()
                .build()
                .map_err(|_| AgentError::Gemini("replay transport failed".to_owned()))?;
            let client = GeminiConfig::new("replay-fixture")
                .with_base_url(base)
                .connect(rig_reqwest::ReqwestClient::from(http));
            return Ok(client.completion(GEMINI_MODEL));
        }
        let client = Gemini::from_env().map_err(|_| AgentError::MissingGeminiKey)?;
        Ok(client.completion(GEMINI_MODEL))
    }
}

async fn run_approver(desk: &Desk, model: Model<GenerateContent>) -> Result<(), AgentError> {
    let agent = AgentBuilder::new(model)
        .name("approver")
        .preamble(APPROVER_PREAMBLE)
        .tool(ListBills { desk: desk.clone() })
        .tool(ApproveBill { desk: desk.clone() })
        .default_max_turns(10)
        .temperature(0.0)
        .record_content_telemetry(false)
        .build();
    let span = tracing::info_span!(
        "agent.decide",
        sentry.name = "agent.decide",
        sentry.op = "agent.decide",
        agent.id = "approver",
        decision.kind = "approve",
    );
    async move { agent.prompt(APPROVER_PROMPT).max_turns(10).await }
        .instrument(span)
        .await
        .map(|_| ())
        .map_err(|err| AgentError::Gemini(redact(&err.to_string())))
}

async fn run_payer(desk: &Desk, model: Model<GenerateContent>) -> Result<(), AgentError> {
    let prompt = payer_prompt(desk);
    let agent = AgentBuilder::new(model)
        .name("payer")
        .preamble(PAYER_PREAMBLE)
        .tool(ListBills { desk: desk.clone() })
        .tool(PayBill { desk: desk.clone() })
        .default_max_turns(12)
        .temperature(0.0)
        .record_content_telemetry(false)
        .build();
    let span = tracing::info_span!(
        "agent.decide",
        sentry.name = "agent.decide",
        sentry.op = "agent.decide",
        agent.id = "payer",
        decision.kind = "pay",
    );
    async move { agent.prompt(prompt).max_turns(12).await }
        .instrument(span)
        .await
        .map(|_| ())
        .map_err(|err| AgentError::Gemini(redact(&err.to_string())))
}

async fn run_buyer(desk: &Desk, model: Model<GenerateContent>) -> Result<(), AgentError> {
    let agent = AgentBuilder::new(model)
        .name("buyer")
        .preamble(BUYER_PREAMBLE)
        .tool(BuyFlour { desk: desk.clone() })
        .default_max_turns(3)
        .temperature(0.0)
        .record_content_telemetry(false)
        .build();
    let span = tracing::info_span!(
        "agent.decide",
        sentry.name = "agent.decide",
        sentry.op = "agent.decide",
        agent.id = "buyer",
        decision.kind = "trade",
        order.id = tracing::field::Empty,
    );
    async move { agent.prompt(BUYER_PROMPT).max_turns(3).await }
        .instrument(span)
        .await
        .map(|_| ())
        .map_err(|err| AgentError::Gemini(redact(&err.to_string())))
}

fn payer_prompt(desk: &Desk) -> String {
    let mut lines = vec![
        "Pay the open bill ids from list_bills that fit the budget.".to_owned(),
        "Call list_bills, then pay_bill once for each open bill you are paying.".to_owned(),
    ];
    if desk.fault_enabled(Fault::DoublePay) {
        lines.push(
            "The double-pay fault is on. Also call pay_bill once with the settled bill id."
                .to_owned(),
        );
    }
    if desk.fault_enabled(Fault::PayWithoutApproval) {
        lines.push(
            "The no-approval fault is on. Call pay_bill even when the bill has no approval."
                .to_owned(),
        );
    }
    if desk.fault_enabled(Fault::ExceedBudget) {
        lines.push(
            "The exceed-budget fault is on. Call pay_bill even when the amount is over the remaining budget."
                .to_owned(),
        );
    }
    if desk.fault_enabled(Fault::SwappedVendor) {
        lines.push(
            "The swapped-vendor fault is on. Call pay_bill; the tool pays an aliased vendor."
                .to_owned(),
        );
    }
    if desk.fault_enabled(Fault::HiddenMemo) {
        lines.push(
            "The hidden-memo fault is on. Call pay_bill; the tool follows a memo instruction when the bill has one."
                .to_owned(),
        );
    }
    lines.join(" ")
}

/// Removes query credentials and long provider dumps from `text`.
fn redact(text: &str) -> String {
    let mut out = text.to_owned();
    while let Some(start) = out.find("key=") {
        let value_start = start + "key=".len();
        let value_end = out[value_start..]
            .find(['&', ' ', '"', '\'', '\n', '\r'])
            .map_or(out.len(), |end| value_start + end);
        out.replace_range(start..value_end, "credential=[redacted]");
    }
    while let Some(start) = out.find("AIza") {
        let end = out[start..]
            .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_')
            .map_or(out.len(), |offset| start + offset);
        if end - start < 8 {
            break;
        }
        out.replace_range(start..end, "[redacted]");
    }
    const LIMIT: usize = 300;
    if out.len() > LIMIT {
        out.truncate(LIMIT);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use ledger_core::dataset::{load_company, CompanyBooks};

    use crate::capture::Seen;
    use crate::fault::FaultConfig;
    use crate::replay;

    #[test]
    fn redaction_strips_a_query_key() {
        let raw = "GET /v1?key=super-secret-value&alt=json failed";
        let cleaned = redact(raw);
        assert!(!cleaned.contains("super-secret-value"));
        assert!(cleaned.contains("credential=[redacted]"));
        assert!(cleaned.contains("alt=json"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rig_loop_executes_the_replayed_gemini_tool_call() {
        let capture = crate::capture::Capture::new();
        let _guard = capture.guard();
        assert_eq!(GEMINI_MODEL, "gemini-3.8-flash");
        let books = load_fixture("company-fixture");
        let desk = Desk::new(books, money("150.00"), FaultConfig::new()).expect("desk");
        desk.replay_gemini(replay::serve());
        desk.approve_bill("bill-open");
        let report = desk.run_cycle().await.expect("cycle");
        drop(_guard);
        assert!(
            report
                .payments
                .iter()
                .any(|payment| payment.bill_local == "bill-open"),
            "rig should have called pay_bill for the recorded tool call"
        );
        let spans = capture.spans();
        let tool = spans
            .iter()
            .find(|span| span.name == "execute_tool")
            .expect("rig execute_tool span");
        assert!(
            Seen::chain(&spans, tool).contains(&"agent.decide"),
            "tool execution should stay inside the payer decision span"
        );
    }

    fn load_fixture(name: &str) -> CompanyBooks {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(name);
        load_company(&path).unwrap_or_else(|err| panic!("fixture {name}: {err}"))
    }

    fn money(raw: &str) -> ledger_core::dataset::Money {
        raw.parse().unwrap_or_else(|_| panic!("money {raw}"))
    }
}
