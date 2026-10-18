//! Test subscriber that records span names, parents, and fields.

use std::fmt;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

#[derive(Clone, Default)]
pub struct Capture {
    spans: Arc<Mutex<Vec<Seen>>>,
}

#[derive(Clone, Debug)]
pub struct Seen {
    pub id: Id,
    pub parent: Option<Id>,
    pub name: &'static str,
    pub fields: Vec<(String, String)>,
}

impl Capture {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn guard(&self) -> tracing::subscriber::DefaultGuard {
        let subscriber = tracing_subscriber::registry().with(self.clone());
        tracing::subscriber::set_default(subscriber)
    }

    pub fn spans(&self) -> Vec<Seen> {
        self.spans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Seen {
    pub fn value(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .rev()
            .find(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn chain<'a>(spans: &'a [Seen], start: &Seen) -> Vec<&'a str> {
        let mut names = vec![start.name];
        let mut parent = start.parent.clone();
        for _ in 0..16 {
            let Some(id) = parent else {
                break;
            };
            let Some(next) = spans.iter().find(|span| span.id == id) else {
                break;
            };
            names.push(next.name);
            parent = next.parent.clone();
        }
        names
    }
}

impl<S> Layer<S> for Capture
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let mut fields = Vec::new();
        attrs.record(&mut FieldList(&mut fields));
        let parent = ctx.current_span().id().cloned();
        self.spans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Seen {
                id: id.clone(),
                parent,
                name: attrs.metadata().name(),
                fields,
            });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut fields = Vec::new();
        values.record(&mut FieldList(&mut fields));
        let mut spans = self
            .spans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(span) = spans.iter_mut().find(|span| &span.id == id) {
            span.fields.extend(fields);
        }
    }
}

struct FieldList<'a>(&'a mut Vec<(String, String)>);

impl Visit for FieldList<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0.push((field.name().to_owned(), format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push((field.name().to_owned(), value.to_owned()));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.push((field.name().to_owned(), value.to_string()));
    }
}
