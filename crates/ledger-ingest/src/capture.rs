//! Tracing layer that forwards every span into ingest.
//!
//! There is no sample rate. A level filter may be wrapped around the layer by
//! the process subscriber; that gate is not sampling. Every span the layer
//! receives is handed to the sink.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

/// One closed tracing span, ready to normalise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapturedSpan {
    /// Span name, such as `agent.decide` or `exchange.submit_order`.
    pub name: &'static str,
    /// Tracing span id. Stable only inside this process.
    pub span_id: u64,
    /// Parent span id, when the span was created under another span.
    pub parent_span_id: Option<u64>,
    /// When the span opened.
    pub started_at: DateTime<Utc>,
    /// When the span closed. This is the observation time stored on the event.
    pub closed_at: DateTime<Utc>,
    /// Span fields. Keys are tracing field names.
    pub fields: BTreeMap<String, String>,
}

impl CapturedSpan {
    /// Builds a captured span whose close time equals `started_at`.
    #[must_use]
    pub fn new(
        name: &'static str,
        span_id: u64,
        parent_span_id: Option<u64>,
        started_at: DateTime<Utc>,
        fields: BTreeMap<String, String>,
    ) -> Self {
        Self {
            name,
            span_id,
            parent_span_id,
            started_at,
            closed_at: started_at,
            fields,
        }
    }
}

/// Receives closed spans. Implementations must not sample.
pub trait SpanSink: Send + Sync + 'static {
    /// Accepts one closed span.
    fn on_span(&self, span: CapturedSpan);
}

impl SpanSink for mpsc::Sender<CapturedSpan> {
    fn on_span(&self, span: CapturedSpan) {
        let _ = self.send(span);
    }
}

/// Tracing layer that forwards every span it is shown.
///
/// The layer does not drop spans to meet a sample rate. If the sink is gone,
/// the span cannot be delivered; that is shutdown, not sampling.
#[derive(Clone)]
#[must_use = "an ingest layer records nothing until it is installed on a subscriber"]
pub struct IngestLayer {
    sink: Arc<dyn SpanSink>,
}

impl IngestLayer {
    /// Wraps `sink`. Every closed span is passed to [`SpanSink::on_span`].
    #[must_use]
    pub fn from_sink(sink: impl SpanSink) -> Self {
        Self {
            sink: Arc::new(sink),
        }
    }

    /// Layer plus the channel that receives every closed span.
    #[must_use]
    pub fn pair() -> (Self, mpsc::Receiver<CapturedSpan>) {
        let (sender, receiver) = mpsc::channel();
        (Self::from_sink(sender), receiver)
    }
}

#[derive(Clone)]
struct SpanState {
    fields: BTreeMap<String, String>,
    parent_span_id: Option<u64>,
    started_at: DateTime<Utc>,
}

impl<S> Layer<S> for IngestLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut fields = BTreeMap::new();
        attrs.record(&mut FieldVisitor(&mut fields));
        let parent_span_id = span
            .parent()
            .map(|parent| parent.id())
            .or_else(|| ctx.current_span().id().cloned())
            .filter(|parent| parent != id)
            .map(|parent| parent.into_u64());
        span.extensions_mut().insert(SpanState {
            fields,
            parent_span_id,
            started_at: utc_now(),
        });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else {
            return;
        };
        let mut extensions = span.extensions_mut();
        let Some(state) = extensions.get_mut::<SpanState>() else {
            return;
        };
        values.record(&mut FieldVisitor(&mut state.fields));
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else {
            return;
        };
        let Some(state) = span.extensions().get::<SpanState>().cloned() else {
            return;
        };
        self.sink.on_span(CapturedSpan {
            name: span.name(),
            span_id: id.into_u64(),
            parent_span_id: state.parent_span_id,
            started_at: state.started_at,
            closed_at: utc_now(),
            fields: state.fields,
        });
    }
}

struct FieldVisitor<'a>(&'a mut BTreeMap<String, String>);

impl Visit for FieldVisitor<'_> {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(field.name().to_owned(), value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

fn utc_now() -> DateTime<Utc> {
    utc_from(SystemTime::now())
}

fn utc_from(time: SystemTime) -> DateTime<Utc> {
    let Ok(duration) = time.duration_since(SystemTime::UNIX_EPOCH) else {
        return DateTime::UNIX_EPOCH;
    };
    let nanos = duration.subsec_nanos();
    let seconds = i64::try_from(duration.as_secs()).unwrap_or(i64::MAX);
    DateTime::from_timestamp(seconds, nanos).unwrap_or(DateTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::Registry;

    #[test]
    fn every_info_span_is_forwarded() {
        let (layer, receiver) = IngestLayer::pair();
        tracing::subscriber::with_default(Registry::default().with(layer), || {
            for _ in 0..100 {
                let span = tracing::info_span!("agent.decide", agent.id = "payer");
                drop(span.enter());
            }
        });
        let spans: Vec<_> = receiver.try_iter().collect();
        assert_eq!(spans.len(), 100);
        assert!(spans.iter().all(|span| span.name == "agent.decide"));
    }

    #[test]
    fn agent_and_exchange_spans_keep_parent_and_fields() {
        let (layer, receiver) = IngestLayer::pair();
        tracing::subscriber::with_default(Registry::default().with(layer), || {
            let decide = tracing::info_span!(
                "agent.decide",
                agent.id = "payer",
                decision.kind = "pay",
                bill.id = "bill-1"
            );
            let _decide = decide.enter();
            let pay = tracing::info_span!(
                "agent.pay_bill",
                agent.id = "payer",
                bill.id = tracing::field::Empty,
                link.approval_id = "apr-1"
            );
            pay.record("bill.id", "bill-1");
            drop(pay.enter());

            let _submit =
                tracing::info_span!("exchange.submit_order", agent.id = "buyer", order.id = "7")
                    .entered();
            let _matched = tracing::info_span!("exchange.match", order.id = "7").entered();
            let _filled =
                tracing::info_span!("exchange.fill", fill.id = "1", order.id = "7").entered();
            let _settled = tracing::info_span!("exchange.settle", fill.id = "1").entered();
            let _approved = tracing::info_span!(
                "agent.approve",
                agent.id = "approver",
                approval.id = "apr-1",
                bill.id = "bill-1"
            )
            .entered();
            let _agent_settled = tracing::info_span!(
                "agent.settle",
                agent.id = "buyer",
                order.id = "7",
                fill.id = "1",
                link.order_id = "7",
                link.fill_id = "1"
            )
            .entered();
        });

        let spans: Vec<_> = receiver.try_iter().collect();
        let names: Vec<_> = spans.iter().map(|span| span.name).collect();
        assert!(names.contains(&"agent.decide"));
        assert!(names.contains(&"agent.pay_bill"));
        assert!(names.contains(&"agent.approve"));
        assert!(names.contains(&"agent.settle"));
        assert!(names.contains(&"exchange.submit_order"));
        assert!(names.contains(&"exchange.match"));
        assert!(names.contains(&"exchange.fill"));
        assert!(names.contains(&"exchange.settle"));

        let decide = spans
            .iter()
            .find(|span| span.name == "agent.decide")
            .expect("decide span");
        let pay = spans
            .iter()
            .find(|span| span.name == "agent.pay_bill")
            .expect("pay span");
        assert_eq!(pay.parent_span_id, Some(decide.span_id));
        assert_eq!(
            pay.fields.get("bill.id").map(String::as_str),
            Some("bill-1")
        );
        assert_eq!(
            pay.fields.get("link.approval_id").map(String::as_str),
            Some("apr-1")
        );
        assert_eq!(
            spans
                .iter()
                .find(|span| span.name == "exchange.submit_order")
                .expect("submit span")
                .parent_span_id,
            Some(decide.span_id)
        );
    }
}
