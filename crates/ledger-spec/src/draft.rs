//! Gemini draft of one catalogue Pattern.
//!
//! The request is the Interactions API (`gemini-3.8-flash`, JSON schema). The
//! model returns a Pattern id and that Pattern's parameter values. It is not
//! asked for prose, and thought steps are ignored.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ledger_core::{LedgerKind, BILL_ATTRIBUTE};
use sentinel_compile::catalogue::{catalogue, ParameterKind};
use serde_json::{Map, Value};

/// Model named by the Gemini Interactions text-generation docs.
pub const GEMINI_MODEL: &str = "gemini-3.8-flash";

/// Policy sentence used for the saved Gemini and Jev fixtures.
pub const FIXTURE_POLICY: &str =
    "After an agent pays a bill, no second payment may occur on that bill until a credit closes it.";

/// One catalogue parameter value in a Gemini draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftValue {
    /// Attribute key or event kind.
    Text(String),
    /// Lookback or count threshold.
    Integer(u64),
}

/// A Pattern id plus the parameters Gemini bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternDraft {
    pattern_id: String,
    parameters: BTreeMap<String, DraftValue>,
}

impl PatternDraft {
    /// Returns the catalogue id the draft names.
    #[must_use]
    pub fn pattern_id(&self) -> &str {
        &self.pattern_id
    }

    /// Returns the bound parameters, in name order.
    #[must_use]
    pub fn parameters(&self) -> &BTreeMap<String, DraftValue> {
        &self.parameters
    }
}

/// Body of `POST /v1beta/interactions` for `sentence`.
///
/// The body has no API key. `store` is false.
#[must_use]
pub fn draft_request(sentence: &str) -> String {
    serde_json::to_string(&draft_request_value(sentence)).expect("draft request is finite JSON")
}

fn draft_request_value(sentence: &str) -> Value {
    serde_json::json!({
        "model": GEMINI_MODEL,
        "store": false,
        "input": prompt(sentence),
        "response_format": {
            "type": "text",
            "mime_type": "application/json",
            "schema": schema()
        },
        "generation_config": {
            "thinking_level": "low"
        }
    })
}

fn prompt(sentence: &str) -> String {
    format!(
        "Draft one catalogue Pattern for the policy. Return only the JSON object. Do not add prose.\n\n\
Policy:\n{sentence}\n\n\
Use pattern_id exactly as listed. Include only the parameters that pattern declares, under those exact names. Omit every other field.\n\n\
A parameter whose kind is attribute-key must be \"{bill}\".\n\
A parameter whose kind is selector must be one of these event kinds:\n{kinds}\
A parameter whose kind is sequence-lookback-bound or count-threshold must be an integer.\n\n\
Patterns:\n{patterns}",
        bill = BILL_ATTRIBUTE,
        kinds = kind_lines(),
        patterns = catalogue_prompt(),
    )
}

fn kind_lines() -> String {
    let mut lines = String::new();
    for kind in ledger_kinds() {
        let _ = writeln!(lines, "- {} ({})", kind.event_kind(), kind_gloss(kind));
    }
    lines
}

fn kind_gloss(kind: LedgerKind) -> &'static str {
    match kind {
        LedgerKind::AgentPayBill => "the agent pays the bill",
        LedgerKind::ObligationSettled => "the exchange records the bill's first settlement",
        LedgerKind::ExtraPayment => {
            "the exchange records another payment on an already settled bill"
        }
        LedgerKind::Credit => "the exchange records a credit that closes the bill",
    }
}

pub(crate) fn ledger_kinds() -> [LedgerKind; 4] {
    [
        LedgerKind::AgentPayBill,
        LedgerKind::ObligationSettled,
        LedgerKind::ExtraPayment,
        LedgerKind::Credit,
    ]
}

fn catalogue_prompt() -> String {
    let mut text = String::new();
    for declaration in catalogue() {
        let _ = writeln!(
            text,
            "- {} ({}): {}",
            declaration.id.as_str(),
            declaration.shape.as_str(),
            declaration.summary
        );
        for parameter in declaration.parameters {
            let _ = writeln!(
                text,
                "  - {} ({}): {}",
                parameter.name,
                parameter.kind.as_str(),
                parameter.description
            );
        }
    }
    text
}

fn schema() -> Value {
    let kinds = event_kind_names();
    let mut branches = Vec::new();
    for declaration in catalogue() {
        let mut properties = Map::new();
        let mut required = vec!["pattern_id".to_owned()];
        properties.insert(
            "pattern_id".to_owned(),
            serde_json::json!({ "type": "string", "enum": [declaration.id.as_str()] }),
        );
        for parameter in declaration.parameters {
            let spec = match parameter.kind {
                ParameterKind::AttributeKey => {
                    serde_json::json!({ "type": "string", "enum": [BILL_ATTRIBUTE] })
                }
                ParameterKind::Selector => serde_json::json!({ "type": "string", "enum": kinds }),
                ParameterKind::SequenceLookbackBound | ParameterKind::CountThreshold => {
                    serde_json::json!({ "type": "integer" })
                }
                _ => serde_json::json!({ "type": "string" }),
            };
            properties.insert(parameter.name.to_owned(), spec);
            required.push(parameter.name.to_owned());
        }
        branches.push(serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required
        }));
    }
    serde_json::json!({ "anyOf": branches })
}

fn event_kind_names() -> Vec<&'static str> {
    ledger_kinds()
        .iter()
        .map(|kind| kind.event_kind())
        .collect()
}

/// Reads a draft from a raw Interactions API body.
///
/// Only `model_output` text is read. Thought steps are ignored. Returns
/// [`None`] when the body is not a completed interaction or the text is not
/// the draft object.
#[must_use = "an unread draft is dropped"]
pub fn draft_from_interaction(body: &str) -> Option<PatternDraft> {
    let value: Value = serde_json::from_str(body).ok()?;
    if value.get("object").and_then(Value::as_str) != Some("interaction") {
        return None;
    }
    if value.get("status").and_then(Value::as_str) != Some("completed") {
        return None;
    }
    let text = model_output_text(&value)?;
    draft_from_model_text(&text)
}

fn model_output_text(value: &Value) -> Option<String> {
    let steps = value.get("steps")?.as_array()?;
    let mut parts = Vec::new();
    for step in steps {
        if step.get("type").and_then(Value::as_str) != Some("model_output") {
            continue;
        }
        let Some(content) = step.get("content").and_then(Value::as_array) else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("text") {
                continue;
            }
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                parts.push(text);
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.concat())
    }
}

fn draft_from_model_text(text: &str) -> Option<PatternDraft> {
    let value: Value = serde_json::from_str(strip_fences(text)).ok()?;
    let pattern_id = value.get("pattern_id").and_then(Value::as_str)?.to_owned();
    let object = value.as_object()?;
    let mut parameters = BTreeMap::new();
    for (key, item) in object {
        if key == "pattern_id" || item.is_null() {
            continue;
        }
        let parsed = if let Some(text) = item.as_str() {
            DraftValue::Text(text.to_owned())
        } else {
            DraftValue::Integer(item.as_u64()?)
        };
        parameters.insert(key.clone(), parsed);
    }
    Some(PatternDraft {
        pattern_id,
        parameters,
    })
}

fn strip_fences(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let rest = rest.trim_start_matches("json").trim_start();
    rest.strip_suffix("```").unwrap_or(rest).trim()
}

pub(crate) fn parameters_json(draft: &PatternDraft) -> Value {
    let mut object = Map::new();
    for (name, value) in &draft.parameters {
        let json = match value {
            DraftValue::Text(text) => Value::String(text.clone()),
            DraftValue::Integer(number) => Value::from(*number),
        };
        object.insert(name.clone(), json);
    }
    Value::Object(object)
}
