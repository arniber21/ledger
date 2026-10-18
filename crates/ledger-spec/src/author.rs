//! Authoring pipeline: sentence, Gemini draft, catalogue check, Jev match.
//!
//! Catalogue validation runs before Jev. A failed or low-scoring match is not
//! an acceptance. Accepted drafts become a running monitor.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use ledger_core::AgentId;
use ledger_jev::{map_choice_exchange, BindingVerdict, JevCallFailure};
use serde_json::{json, Value};

use crate::check::DRAFT_MATCH_QUESTION;
use crate::draft::{draft_from_interaction, DraftValue};
use crate::monitor::{start_monitor, RunningMonitor};
use crate::reason::RejectReason;

/// Why a Gemini call produced no usable body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftFailure {
    /// The call exceeded its limit.
    Timeout,
    /// The call failed before an HTTP status arrived.
    Transport,
    /// Gemini returned a non-success status.
    Http {
        /// HTTP status code.
        status: u16,
    },
    /// The response body was not UTF-8.
    Utf8,
}

/// Inputs to one authoring run.
///
/// `gemini` is the raw Interactions API body. `jev` is the raw System One
/// body. `jev` is ignored when the draft is not catalogue-valid.
pub struct AuthoringInput<'a> {
    /// Plain-English policy.
    pub sentence: &'a str,
    /// Agent whose tool-call log the monitor binds.
    pub agent: &'a AgentId,
    /// Raw Gemini body, or why the call produced none.
    pub gemini: Result<&'a str, DraftFailure>,
    /// Raw Jev body, or why the call produced none.
    pub jev: Result<&'a str, JevCallFailure>,
}

/// One step the UI streams.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum AuthoringStep {
    /// The policy sentence.
    Sentence {
        /// Policy text.
        text: String,
    },
    /// The parsed Gemini draft, before acceptance.
    Draft {
        /// Catalogue id the draft named.
        pattern_id: String,
        /// Parameter values the draft bound.
        parameters: BTreeMap<String, DraftValue>,
    },
    /// The Jev Same / Different / Unknown judgment.
    Check {
        /// Mapped verdict. Thresholds live in `ledger-jev`.
        verdict: BindingVerdict,
    },
    /// The draft was catalogue-valid and Jev returned Same.
    Accepted {
        /// Catalogue id now running.
        pattern_id: String,
    },
    /// The draft was not accepted.
    Rejected {
        /// Reason code. No model prose.
        reason: RejectReason,
    },
}

/// The steps of one run, and the monitor when the draft was accepted.
#[derive(Debug)]
#[must_use]
pub struct AuthoringOutcome {
    steps: Vec<AuthoringStep>,
    monitor: Option<RunningMonitor>,
}

impl AuthoringOutcome {
    /// Returns the steps in stream order.
    #[must_use]
    pub fn steps(&self) -> &[AuthoringStep] {
        &self.steps
    }

    /// Returns the monitor when the draft was accepted.
    #[must_use]
    pub fn monitor(&self) -> Option<&RunningMonitor> {
        self.monitor.as_ref()
    }

    /// Takes the monitor when the draft was accepted.
    #[must_use]
    pub fn into_monitor(self) -> Option<RunningMonitor> {
        self.monitor
    }

    /// Encodes the steps as `text/event-stream` frames.
    ///
    /// Events are `sentence`, `draft`, `check`, `accepted`, and `rejected`.
    #[must_use]
    pub fn encode_sse(&self) -> String {
        let mut out = String::new();
        for step in &self.steps {
            let (event, data) = step.sse();
            let _ = writeln!(out, "event: {event}");
            let _ = writeln!(out, "data: {data}");
            out.push('\n');
        }
        out
    }
}

impl AuthoringStep {
    fn sse(&self) -> (&'static str, String) {
        let (event, data) = match self {
            Self::Sentence { text } => ("sentence", json!({ "text": text })),
            Self::Draft {
                pattern_id,
                parameters,
            } => (
                "draft",
                json!({
                    "pattern_id": pattern_id,
                    "parameters": parameter_object(parameters)
                }),
            ),
            Self::Check { verdict } => ("check", json!({ "verdict": match_label(*verdict) })),
            Self::Accepted { pattern_id } => (
                "accepted",
                json!({ "pattern_id": pattern_id, "monitor": "running" }),
            ),
            Self::Rejected { reason } => ("rejected", reason.sse_data()),
        };
        (
            event,
            serde_json::to_string(&data).expect("authoring step is finite JSON"),
        )
    }
}

fn parameter_object(parameters: &BTreeMap<String, DraftValue>) -> Value {
    let mut object = serde_json::Map::new();
    for (name, value) in parameters {
        let json = match value {
            DraftValue::Text(text) => Value::String(text.clone()),
            DraftValue::Integer(number) => Value::from(*number),
        };
        object.insert(name.clone(), json);
    }
    Value::Object(object)
}

fn match_label(verdict: BindingVerdict) -> &'static str {
    match verdict {
        BindingVerdict::Same => "Same",
        BindingVerdict::Different => "Different",
        BindingVerdict::Unknown => "Unknown",
    }
}

/// Runs catalogue validation, then the Jev match check.
///
/// An empty sentence, a failed Gemini call, an unreadable body, or a draft
/// that is not catalogue-valid is rejected before Jev. Jev `Unknown` rejects
/// with `match_inconclusive` and does not start a monitor.
#[must_use = "the outcome holds the running monitor"]
pub fn author(input: AuthoringInput<'_>) -> AuthoringOutcome {
    let mut steps = vec![AuthoringStep::Sentence {
        text: input.sentence.to_owned(),
    }];
    if input.sentence.trim().is_empty() {
        return reject(&mut steps, RejectReason::EmptyPolicy);
    }
    let body = match input.gemini {
        Ok(body) => body,
        Err(
            DraftFailure::Timeout
            | DraftFailure::Transport
            | DraftFailure::Http { .. }
            | DraftFailure::Utf8,
        ) => return reject(&mut steps, RejectReason::DraftUnavailable),
    };
    let Some(draft) = draft_from_interaction(body) else {
        return reject(&mut steps, RejectReason::DraftUnreadable);
    };
    steps.push(AuthoringStep::Draft {
        pattern_id: draft.pattern_id().to_owned(),
        parameters: draft.parameters().clone(),
    });
    let monitor = match start_monitor(&draft, input.agent) {
        Ok(monitor) => monitor,
        Err(reason) => return reject(&mut steps, reason),
    };
    let verdict = map_choice_exchange(input.jev, DRAFT_MATCH_QUESTION);
    steps.push(AuthoringStep::Check { verdict });
    match verdict {
        BindingVerdict::Same => {
            let pattern_id = monitor.pattern_id().to_owned();
            steps.push(AuthoringStep::Accepted { pattern_id });
            AuthoringOutcome {
                steps,
                monitor: Some(monitor),
            }
        }
        BindingVerdict::Different => reject(&mut steps, RejectReason::DraftDiffers),
        BindingVerdict::Unknown => reject(&mut steps, RejectReason::MatchInconclusive),
    }
}

fn reject(steps: &mut Vec<AuthoringStep>, reason: RejectReason) -> AuthoringOutcome {
    steps.push(AuthoringStep::Rejected { reason });
    AuthoringOutcome {
        steps: std::mem::take(steps),
        monitor: None,
    }
}
