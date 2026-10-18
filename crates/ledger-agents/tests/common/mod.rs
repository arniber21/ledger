//! Shared loaders and a tracing subscriber that records spans for agent tests.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ledger_agents::{CycleReport, Desk, Fault, FaultConfig, PayOutcome};
use ledger_core::dataset::{load_company, CompanyBooks, Money};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Layer;

#[derive(Clone, Debug)]
pub struct CapturedSpan {
    pub id: Id,
    pub parent: Option<Id>,
    pub name: &'static str,
    pub fields: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct CapturedEvent {
    pub name: &'static str,
    pub fields: HashMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct CaptureState {
    pub spans: Vec<CapturedSpan>,
    pub events: Vec<CapturedEvent>,
    /// Span ids in the order the spans closed. Ingest stamps `observed_at` at close.
    pub closed: Vec<Id>,
}

#[derive(Clone, Default)]
pub struct Capture {
    inner: Arc<Mutex<CaptureState>>,
}

impl Capture {
    pub fn snapshot(&self) -> CaptureState {
        self.inner.lock().expect("capture lock").clone()
    }
}

impl<S> Layer<S> for Capture
where
    S: Subscriber + for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let parent = if let Some(parent) = attrs.parent() {
            Some(parent.clone())
        } else if attrs.is_contextual() {
            ctx.current_span().id().cloned()
        } else {
            None
        };
        let mut fields = HashMap::new();
        attrs.record(&mut Visitor {
            fields: &mut fields,
        });
        self.inner
            .lock()
            .expect("capture lock")
            .spans
            .push(CapturedSpan {
                id: id.clone(),
                parent,
                name: attrs.metadata().name(),
                fields,
            });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut fields = HashMap::new();
        values.record(&mut Visitor {
            fields: &mut fields,
        });
        let mut state = self.inner.lock().expect("capture lock");
        if let Some(span) = state.spans.iter_mut().find(|span| span.id == *id) {
            span.fields.extend(fields);
        }
    }

    fn on_close(&self, id: Id, _ctx: Context<'_, S>) {
        self.inner.lock().expect("capture lock").closed.push(id);
    }

    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = HashMap::new();
        event.record(&mut Visitor {
            fields: &mut fields,
        });
        self.inner
            .lock()
            .expect("capture lock")
            .events
            .push(CapturedEvent {
                name: event.metadata().name(),
                fields,
            });
    }
}

struct Visitor<'a> {
    fields: &'a mut HashMap<String, String>,
}

impl Visit for Visitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), normalize(&format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }
}

fn normalize(text: &str) -> String {
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        text[1..text.len() - 1].to_owned()
    } else {
        text.to_owned()
    }
}

pub fn field<'a>(span: &'a CapturedSpan, name: &str) -> &'a str {
    span.fields.get(name).map(String::as_str).unwrap_or("")
}

pub fn event_text(event: &CapturedEvent) -> &str {
    event
        .fields
        .get("message")
        .map(String::as_str)
        .unwrap_or(event.name)
}

pub fn event_field<'a>(event: &'a CapturedEvent, name: &str) -> &'a str {
    event.fields.get(name).map(String::as_str).unwrap_or("")
}

/// Drives the desk tools directly and records their spans.
///
/// This is the fault-test path. The live cycle is [`Desk::run_cycle`], which
/// asks Rig to call the same tools.
pub fn run_cycle(desk: &mut Desk) -> (CycleReport, CaptureState) {
    let capture = Capture::default();
    let layer = capture.clone();
    let report =
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
            drive_tools(desk)
        });
    (report, capture.snapshot())
}

fn drive_tools(desk: &Desk) -> CycleReport {
    desk.begin_cycle();
    let no_approval = desk.fault_enabled(Fault::PayWithoutApproval);
    let double_pay = desk.fault_enabled(Fault::DoublePay);
    let open = desk.open_bill_ids();
    if !no_approval {
        let span = tracing::info_span!(
            "agent.decide",
            sentry.name = "agent.decide",
            sentry.op = "agent.decide",
            agent.id = "approver",
            decision.kind = "approve",
        );
        let _entered = span.enter();
        for id in &open {
            desk.approve_bill(id);
        }
        if double_pay {
            if let Some(id) = desk.next_settled_bill() {
                desk.approve_bill(&id);
            }
        }
    }
    {
        let span = tracing::info_span!(
            "agent.decide",
            sentry.name = "agent.decide",
            sentry.op = "agent.decide",
            agent.id = "payer",
            decision.kind = "pay",
        );
        let _entered = span.enter();
        for id in open {
            match desk.pay_bill(&id) {
                PayOutcome::Withheld(_) => break,
                PayOutcome::Paid(payment) if payment.faults.contains(&Fault::ExceedBudget) => {
                    break;
                }
                PayOutcome::Paid(_) | PayOutcome::Inconclusive(_) | _ => {}
            }
        }
        if double_pay {
            if let Some(id) = desk.next_settled_bill() {
                let _ = desk.pay_bill(&id);
            }
        }
    }
    desk.seed_liquidity().expect("resting ask");
    let span = tracing::info_span!(
        "agent.decide",
        sentry.name = "agent.decide",
        sentry.op = "agent.decide",
        agent.id = "buyer",
        decision.kind = "trade",
        order.id = tracing::field::Empty,
    );
    let _entered = span.enter();
    desk.buy_flour().expect("buyer fill");
    drop(_entered);
    desk.report()
}

pub fn load_fixture(name: &str) -> CompanyBooks {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name);
    load_company(&path).unwrap_or_else(|err| panic!("fixture {name}: {err}"))
}

pub fn memo_fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/jev")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

pub fn fixture_desk(faults: FaultConfig) -> Desk {
    Desk::new(load_fixture("company-fixture"), money("150.00"), faults).expect("desk")
}

pub fn money(raw: &str) -> Money {
    raw.parse().unwrap_or_else(|_| panic!("money {raw}"))
}

pub fn payment<'a>(report: &'a CycleReport, local: &str) -> &'a ledger_agents::Payment {
    report
        .payments
        .iter()
        .find(|payment| payment.bill_local == local)
        .unwrap_or_else(|| panic!("missing payment {local}; got {report:?}"))
}

pub fn assert_no_fraud(report: &CycleReport, state: &CaptureState) {
    let blob = format!("{report:?} {state:?}");
    assert!(
        !blob.to_ascii_lowercase().contains("fraud"),
        "a simulated finding used the word fraud: {blob}"
    );
}
