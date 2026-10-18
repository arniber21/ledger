//! Shared loaders and a tracing subscriber that records spans for agent tests.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ledger_agents::{CycleReport, Desk, FaultConfig};
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

pub fn run_cycle(desk: &mut Desk) -> (CycleReport, CaptureState) {
    let capture = Capture::default();
    let layer = capture.clone();
    let report =
        tracing::subscriber::with_default(tracing_subscriber::registry().with(layer), || {
            desk.run_cycle().expect("cycle")
        });
    (report, capture.snapshot())
}

pub fn load_fixture(name: &str) -> CompanyBooks {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name);
    load_company(&path).unwrap_or_else(|err| panic!("fixture {name}: {err}"))
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
