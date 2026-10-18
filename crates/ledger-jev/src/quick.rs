//! Quick actions: route a phrase and fill typed filters.
//!
//! One System One request asks which handler the phrase is, and asks the
//! filter arguments at the same time. Each argument is a Choice over values
//! the code supplies, plus a Noul for whether the phrase states that
//! argument. The model does not generate free text. Mapping uses
//! [`crate::SAME_PROBABILITY_THRESHOLD`] and
//! [`crate::DIFFERENT_PROBABILITY_THRESHOLD`]. A Choice at or above 0.80
//! fills that argument. A stated Noul of no, at or above 0.80, drops it. A
//! timeout, error, contradiction, or handler score under 0.80 is Unknown.
//! Unknown is not a filled filter and not a pass. This module does not ask
//! for or keep explanation text.
//!
//! Fault injection is a confirmation. This module names the fault. It does
//! not toggle the desk and it does not create a verdict.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value};
use thiserror::Error;

use crate::{
    JevCallFailure, SystemOne, DIFFERENT_PROBABILITY_THRESHOLD, PROBABILITY_SUM_TOLERANCE,
    SAME_PROBABILITY_THRESHOLD,
};

const MAX_PHRASE_CHARS: usize = 280;
const MAX_AGENTS: usize = 8;
const MAX_SPECS: usize = 8;
const MAX_VERDICTS: usize = 12;

const HANDLER: &str = "handler";
const VERDICT: &str = "verdict";
const VERDICT_STATED: &str = "verdict_stated";
const AMOUNT: &str = "amount";
const AMOUNT_STATED: &str = "amount_stated";
const AGENT: &str = "agent";
const AGENT_STATED: &str = "agent_stated";
const SPEC: &str = "spec";
const SPEC_STATED: &str = "spec_stated";
const FAULT: &str = "fault";
const FAULT_STATED: &str = "fault_stated";
const VIEW: &str = "view";
const VIEW_STATED: &str = "view_stated";
const VERDICT_ID: &str = "verdict_id";
const VERDICT_ID_STATED: &str = "verdict_id_stated";

const UNSPECIFIED: &str = "unspecified";

const FAULTS: [&str; 5] = [
    "double-pay",
    "exceed-budget",
    "hidden-memo",
    "no-approval",
    "swapped-vendor",
];

const VIEWS: [&str; 5] = ["dashboard", "inbox", "specs", "traces", "verdicts"];

const VERDICT_OPTIONS: [&str; 3] = ["Inconclusive", "Satisfy", "Violate"];

/// Closed sets the code supplies. The model selects from these lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidates {
    /// Agent ids, such as `agent-2`.
    pub agents: Vec<String>,
    /// Spec ids, such as `ledger.bill.approval-before-payment`.
    pub specs: Vec<String>,
    /// Verdict ids already on the stream. An id outside this list cannot be opened.
    pub verdicts: Vec<String>,
}

impl Candidates {
    /// Agents, specs, and amount bucket the Quick actions catalogue always offers.
    ///
    /// Verdict ids stay empty until a stream supplies them.
    #[must_use]
    pub fn catalogue() -> Self {
        Self {
            agents: vec![
                "agent-1".to_owned(),
                "agent-2".to_owned(),
                "agent-3".to_owned(),
            ],
            specs: vec![
                ledger_core::SPEC_PRECEDENCE.to_owned(),
                ledger_core::SPEC_ABSENCE.to_owned(),
                ledger_core::SPEC_RESPONSE.to_owned(),
            ],
            verdicts: Vec::new(),
        }
    }

    /// An empty extra set.
    #[must_use]
    pub fn none() -> Self {
        Self {
            agents: Vec::new(),
            specs: Vec::new(),
            verdicts: Vec::new(),
        }
    }

    /// Catalogue values first, then sanitized extras. Duplicates drop. The
    /// result is sorted and capped.
    #[must_use]
    pub fn merge(&self, extra: &Self) -> Self {
        Self {
            agents: merge_list(&self.agents, &extra.agents, MAX_AGENTS),
            specs: merge_list(&self.specs, &extra.specs, MAX_SPECS),
            verdicts: merge_list(&[], &extra.verdicts, MAX_VERDICTS),
        }
    }
}

/// One phrase the replay fixtures cover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuickExample {
    /// Fixture file stem.
    pub id: &'static str,
    /// The phrase sent as `state.phrase`.
    pub phrase: &'static str,
}

/// The five phrases recorded under `fixtures/jev/quick/`.
#[must_use]
pub fn quick_examples() -> &'static [QuickExample] {
    &[
        QuickExample {
            id: "violations",
            phrase: "violations over $1,000 by agent-2",
        },
        QuickExample {
            id: "inconclusive",
            phrase: "filter the inbox to inconclusive verdicts on approval before payment",
        },
        QuickExample {
            id: "satisfied",
            phrase: "filter the inbox to satisfied verdicts by agent-1",
        },
        QuickExample {
            id: "unsupported",
            phrase: "what is the weather today",
        },
        QuickExample {
            id: "double-pay",
            phrase: "inject a double-pay fault",
        },
    ]
}

/// Why a phrase was not sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum QuickError {
    /// The phrase is empty after trimming.
    #[error("phrase is empty")]
    Empty,
    /// The phrase is longer than [`MAX_PHRASE_CHARS`].
    #[error("phrase is too long")]
    TooLong,
}

/// Whether the route may be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[must_use]
pub enum QuickStatus {
    /// The handler and its filled arguments cleared the threshold.
    Routed,
    /// Timeout, error, contradiction, or a score under the threshold.
    ///
    /// Not a pass. The action is text search, or missing when a verdict was
    /// requested and no id cleared.
    Unknown,
    /// The handler is `none`. The phrase is text search.
    Unsupported,
    /// The phrase asks to open a verdict that is not in the candidate ids.
    Missing,
}

/// What the palette should do. Filters are closed-set values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[must_use]
pub enum QuickAction {
    /// Inbox search params. Absent fields are not chips.
    FilterInbox {
        /// `Violate`, `Inconclusive`, or `Satisfy`.
        #[serde(skip_serializing_if = "Option::is_none")]
        verdict: Option<String>,
        /// An agent id from the candidate list.
        #[serde(skip_serializing_if = "Option::is_none")]
        agent: Option<String>,
        /// A spec id from the candidate list.
        #[serde(skip_serializing_if = "Option::is_none")]
        spec: Option<String>,
        /// `over-1000` when the phrase asks for that bucket.
        #[serde(skip_serializing_if = "Option::is_none")]
        amount: Option<String>,
    },
    /// Open a verdict id that was in the candidate list.
    OpenVerdict {
        /// Id copied from the candidate list.
        id: String,
    },
    /// Ask the person to confirm a fault toggle. Does not create a verdict.
    InjectFault {
        /// One of the five fault names.
        fault: String,
    },
    /// Navigate to a screen.
    GoTo {
        /// `dashboard`, `inbox`, `traces`, `specs`, or `verdicts`.
        view: String,
    },
    /// Plain text search. Unknown and unsupported phrases use this.
    TextSearch,
    /// The phrase named a verdict the candidate list does not have.
    MissingTarget,
}

impl QuickAction {
    fn empty_filter(&self) -> bool {
        matches!(
            self,
            Self::FilterInbox {
                verdict: None,
                agent: None,
                spec: None,
                amount: None,
            }
        )
    }
}

/// Why the call produced no usable body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[must_use]
pub enum QuickFailure {
    /// The call exceeded the client timeout.
    Timeout,
    /// The call failed before an HTTP status arrived, or no key was configured.
    Transport,
    /// TypeSafe returned a non-success status. The body is not stored.
    Http {
        /// HTTP status code.
        status: u16,
    },
    /// The response body was not UTF-8.
    Utf8,
    /// No System One client was configured.
    Unavailable,
}

/// The request that was sent and the body that came back.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[must_use]
pub struct QuickExchange {
    /// System One request. It has no API key.
    pub request: Value,
    /// System One body with explanation fields removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<Value>,
    /// Set when the call did not return a body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<QuickFailure>,
}

/// A routed phrase, ready for the palette.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[must_use]
pub struct QuickReply {
    /// The trimmed phrase.
    pub phrase: String,
    /// How the action should be treated.
    pub status: QuickStatus,
    /// The palette action. Text search and missing do not carry filter values.
    pub action: QuickAction,
    /// Stored input and output.
    pub exchange: QuickExchange,
}

/// Body of `POST https://api.typesafe.ai/v1/systemone` for one phrase.
///
/// `candidates` is used as given. Callers that want the catalogue merge it
/// first. The body has no API key.
#[must_use]
pub fn quick_request(phrase: &str, candidates: &Candidates) -> String {
    let value = sort_objects(request_value(phrase, candidates));
    serde_json::to_string(&value).expect("quick request is finite JSON")
}

/// Route `phrase` with `client`.
///
/// `None` records Unavailable and returns text search. It does not invent a
/// handler. Candidates are merged with [`Candidates::catalogue`].
///
/// # Errors
///
/// Returns [`QuickError::Empty`] or [`QuickError::TooLong`] before any call.
pub fn route_phrase(
    phrase: &str,
    candidates: &Candidates,
    client: Option<&dyn SystemOne>,
) -> Result<QuickReply, QuickError> {
    let phrase = normalize_phrase(phrase)?;
    let candidates = Candidates::catalogue().merge(candidates);
    let request = quick_request(phrase, &candidates);
    let Some(client) = client else {
        return Ok(failed_reply(phrase, &request, QuickFailure::Unavailable));
    };
    match client.evaluate(&request) {
        Ok(body) => Ok(reply_from_body(phrase, &candidates, &request, &body)),
        Err(failure) => Ok(failed_reply(phrase, &request, failure_kind(failure))),
    }
}

/// Map a finished call. The request is rebuilt from `phrase` and `candidates`
/// after the catalogue merge, so a saved body replays without a socket.
///
/// # Errors
///
/// Returns [`QuickError::Empty`] or [`QuickError::TooLong`] when `phrase` is
/// not usable. A call failure becomes [`QuickStatus::Unknown`].
pub fn map_quick_exchange(
    phrase: &str,
    candidates: &Candidates,
    exchange: Result<&str, JevCallFailure>,
) -> Result<QuickReply, QuickError> {
    let phrase = normalize_phrase(phrase)?;
    let candidates = Candidates::catalogue().merge(candidates);
    let request = quick_request(phrase, &candidates);
    match exchange {
        Ok(body) => Ok(reply_from_body(phrase, &candidates, &request, body)),
        Err(failure) => Ok(failed_reply(phrase, &request, failure_kind(failure))),
    }
}

fn normalize_phrase(phrase: &str) -> Result<&str, QuickError> {
    let phrase = phrase.trim();
    if phrase.is_empty() {
        return Err(QuickError::Empty);
    }
    if phrase.chars().count() > MAX_PHRASE_CHARS {
        return Err(QuickError::TooLong);
    }
    Ok(phrase)
}

fn request_value(phrase: &str, candidates: &Candidates) -> Value {
    serde_json::json!({
        "state": {
            "phrase": phrase,
            "agents": candidates.agents,
            "specs": candidates.specs,
            "verdicts": candidates.verdicts,
            "amount_buckets": ["over-1000"],
            "faults": FAULTS,
            "views": VIEWS
        },
        "model": "jev-latest",
        "questions": questions(candidates)
    })
}

fn questions(candidates: &Candidates) -> Map<String, Value> {
    let mut questions = Map::new();
    questions.insert(
        HANDLER.to_owned(),
        choice(
            "What is the user asking Quick actions to do with `phrase`?",
            [
                (
                    "filter_inbox",
                    "Narrow the Inbox by a verdict outcome, an agent, a spec, or an amount bucket. The phrase is not asking to open one verdict, inject a fault, or change screens.",
                ),
                (
                    "open_verdict",
                    "Open one verdict whose id is in `verdicts`.",
                ),
                (
                    "inject_fault",
                    "Turn on one fault in `faults` on the running desk.",
                ),
                (
                    "go_to_view",
                    "Navigate to one screen in `views`.",
                ),
                (
                    "none",
                    "The phrase is not a request to filter the Inbox, open a verdict, inject a fault, or go to a screen. A question, a comment, or an unsupported request is this option.",
                ),
            ],
        ),
    );
    questions.insert(
        VERDICT.to_owned(),
        choice(
            "Which verdict outcome does `phrase` ask the Inbox to show?",
            [
                (
                    "Violate",
                    "Verdicts that need review, where a policy was violated.",
                ),
                (
                    "Inconclusive",
                    "Verdicts where the evidence did not decide.",
                ),
                ("Satisfy", "Verdicts where the policy held."),
                (
                    UNSPECIFIED,
                    "The phrase does not ask for a verdict outcome.",
                ),
            ],
        ),
    );
    questions.insert(VERDICT_STATED.to_owned(), stated("a verdict outcome"));
    questions.insert(
        AMOUNT.to_owned(),
        choice(
            "Which amount bucket in `amount_buckets` does `phrase` ask for?",
            [
                ("over-1000", "Amounts over $1,000."),
                (UNSPECIFIED, "The phrase does not ask for an amount bucket."),
            ],
        ),
    );
    questions.insert(AMOUNT_STATED.to_owned(), stated("an amount bucket"));
    questions.insert(
        AGENT.to_owned(),
        choice(
            "Which agent in `agents` does `phrase` name?",
            agent_criteria(candidates),
        ),
    );
    questions.insert(AGENT_STATED.to_owned(), stated("an agent in `agents`"));
    questions.insert(
        SPEC.to_owned(),
        choice(
            "Which spec in `specs` does `phrase` name?",
            spec_criteria(candidates),
        ),
    );
    questions.insert(SPEC_STATED.to_owned(), stated("a spec in `specs`"));
    questions.insert(
        FAULT.to_owned(),
        choice(
            "Which fault in `faults` does `phrase` ask to inject?",
            [
                ("double-pay", "Pay a settled obligation a second time."),
                ("no-approval", "Pay without a prior approval."),
                ("exceed-budget", "Pay more than the budget allows."),
                ("swapped-vendor", "Pay a vendor whose name was swapped."),
                (
                    "hidden-memo",
                    "Follow an instruction hidden in a bill memo.",
                ),
                (UNSPECIFIED, "The phrase does not ask to inject a fault."),
            ],
        ),
    );
    questions.insert(FAULT_STATED.to_owned(), stated("a fault in `faults`"));
    questions.insert(
        VIEW.to_owned(),
        choice(
            "Which screen in `views` does `phrase` ask to open?",
            [
                ("dashboard", "The metrics dashboard."),
                ("inbox", "The inbox of verdicts."),
                ("traces", "The live trace screen."),
                ("specs", "The spec authoring screen."),
                ("verdicts", "The verdict screen."),
                (UNSPECIFIED, "The phrase does not ask to open a screen."),
            ],
        ),
    );
    questions.insert(VIEW_STATED.to_owned(), stated("a screen in `views`"));
    questions.insert(
        VERDICT_ID.to_owned(),
        choice(
            "Which id in `verdicts` does `phrase` ask to open?",
            verdict_id_criteria(candidates),
        ),
    );
    questions.insert(
        VERDICT_ID_STATED.to_owned(),
        stated("a verdict id in `verdicts`"),
    );
    questions
}

fn stated(argument: &str) -> Value {
    serde_json::json!({
        "type": "noul",
        "instructions": format!("Does `phrase` ask for {argument}?"),
        "criteria": {
            "true": format!("The phrase asks for {argument}."),
            "false": format!("The phrase does not ask for {argument}.")
        }
    })
}

fn choice<I, K, V>(instructions: &str, criteria: I) -> Value
where
    I: IntoIterator<Item = (K, V)>,
    K: Into<String>,
    V: Into<String>,
{
    let mut map = Map::new();
    for (key, text) in criteria {
        map.insert(key.into(), Value::String(text.into()));
    }
    serde_json::json!({
        "type": "choice",
        "instructions": instructions,
        "criteria": map
    })
}

fn agent_criteria(candidates: &Candidates) -> Vec<(String, String)> {
    let mut criteria: Vec<(String, String)> = candidates
        .agents
        .iter()
        .map(|agent| (agent.clone(), format!("The agent whose id is {agent}.")))
        .collect();
    criteria.push((
        UNSPECIFIED.to_owned(),
        "The phrase does not name an agent in `agents`.".to_owned(),
    ));
    criteria
}

fn spec_criteria(candidates: &Candidates) -> Vec<(String, String)> {
    let mut criteria = Vec::with_capacity(candidates.specs.len() + 1);
    for spec in &candidates.specs {
        let text = match spec.as_str() {
            ledger_core::SPEC_ABSENCE => "No second payment on a settled obligation.".to_owned(),
            ledger_core::SPEC_PRECEDENCE => "A bill's approval precedes its payment.".to_owned(),
            ledger_core::SPEC_RESPONSE => "An order is filled or cancelled.".to_owned(),
            other => format!("The spec whose id is {other}."),
        };
        criteria.push((spec.clone(), text));
    }
    criteria.push((
        UNSPECIFIED.to_owned(),
        "The phrase does not name a spec in `specs`.".to_owned(),
    ));
    criteria
}

fn verdict_id_criteria(candidates: &Candidates) -> Vec<(String, String)> {
    let mut criteria: Vec<(String, String)> = candidates
        .verdicts
        .iter()
        .map(|id| (id.clone(), format!("Open the verdict whose id is {id}.")))
        .collect();
    criteria.push((
        UNSPECIFIED.to_owned(),
        "The phrase does not name an id in `verdicts`.".to_owned(),
    ));
    criteria
}

fn reply_from_body(phrase: &str, candidates: &Candidates, request: &str, body: &str) -> QuickReply {
    let request_value = serde_json::from_str(request).expect("quick request is JSON");
    let Ok(mut response) = serde_json::from_str::<Value>(body) else {
        return QuickReply {
            phrase: phrase.to_owned(),
            status: QuickStatus::Unknown,
            action: QuickAction::TextSearch,
            exchange: QuickExchange {
                request: request_value,
                response: None,
                failure: None,
            },
        };
    };
    strip_explanation(&mut response);
    let (status, action) = compose(candidates, &response);
    QuickReply {
        phrase: phrase.to_owned(),
        status,
        action,
        exchange: QuickExchange {
            request: request_value,
            response: Some(response),
            failure: None,
        },
    }
}

fn failed_reply(phrase: &str, request: &str, failure: QuickFailure) -> QuickReply {
    let request_value = serde_json::from_str(request).expect("quick request is JSON");
    QuickReply {
        phrase: phrase.to_owned(),
        status: QuickStatus::Unknown,
        action: QuickAction::TextSearch,
        exchange: QuickExchange {
            request: request_value,
            response: None,
            failure: Some(failure),
        },
    }
}

fn failure_kind(failure: JevCallFailure) -> QuickFailure {
    match failure {
        JevCallFailure::Timeout => QuickFailure::Timeout,
        JevCallFailure::Transport => QuickFailure::Transport,
        JevCallFailure::Http { status } => QuickFailure::Http { status },
        JevCallFailure::Utf8 => QuickFailure::Utf8,
    }
}

fn compose(candidates: &Candidates, body: &Value) -> (QuickStatus, QuickAction) {
    let answers = body.get("answers").and_then(Value::as_object);
    let handler = map_choice(answer(answers, HANDLER), &handler_options());
    let ChoiceFill::Value(handler) = handler else {
        return (QuickStatus::Unknown, QuickAction::TextSearch);
    };
    match handler.as_str() {
        "none" => (QuickStatus::Unsupported, QuickAction::TextSearch),
        "filter_inbox" => filter_action(candidates, answers),
        "open_verdict" => open_action(candidates, answers),
        "inject_fault" => fault_action(answers),
        "go_to_view" => view_action(answers),
        _ => (QuickStatus::Unknown, QuickAction::TextSearch),
    }
}

fn filter_action(
    candidates: &Candidates,
    answers: Option<&Map<String, Value>>,
) -> (QuickStatus, QuickAction) {
    let action = QuickAction::FilterInbox {
        verdict: argument(
            map_choice(
                answer(answers, VERDICT),
                &with_unspecified(&VERDICT_OPTIONS),
            ),
            map_noul(answer(answers, VERDICT_STATED)),
        )
        .filter(|value| VERDICT_OPTIONS.contains(&value.as_str())),
        agent: argument(
            map_choice(
                answer(answers, AGENT),
                &with_unspecified(&string_slice(&candidates.agents)),
            ),
            map_noul(answer(answers, AGENT_STATED)),
        )
        .filter(|value| candidates.agents.iter().any(|agent| agent == value)),
        spec: argument(
            map_choice(
                answer(answers, SPEC),
                &with_unspecified(&string_slice(&candidates.specs)),
            ),
            map_noul(answer(answers, SPEC_STATED)),
        )
        .filter(|value| candidates.specs.iter().any(|spec| spec == value)),
        amount: argument(
            map_choice(answer(answers, AMOUNT), &with_unspecified(&["over-1000"])),
            map_noul(answer(answers, AMOUNT_STATED)),
        )
        .filter(|value| value == "over-1000"),
    };
    if action.empty_filter() {
        (QuickStatus::Unknown, QuickAction::TextSearch)
    } else {
        (QuickStatus::Routed, action)
    }
}

fn open_action(
    candidates: &Candidates,
    answers: Option<&Map<String, Value>>,
) -> (QuickStatus, QuickAction) {
    let id = argument(
        map_choice(
            answer(answers, VERDICT_ID),
            &with_unspecified(&string_slice(&candidates.verdicts)),
        ),
        map_noul(answer(answers, VERDICT_ID_STATED)),
    )
    .filter(|value| candidates.verdicts.iter().any(|id| id == value));
    match id {
        Some(id) => (QuickStatus::Routed, QuickAction::OpenVerdict { id }),
        None => (QuickStatus::Missing, QuickAction::MissingTarget),
    }
}

fn fault_action(answers: Option<&Map<String, Value>>) -> (QuickStatus, QuickAction) {
    let fault = argument(
        map_choice(answer(answers, FAULT), &with_unspecified(&FAULTS)),
        map_noul(answer(answers, FAULT_STATED)),
    )
    .filter(|value| FAULTS.contains(&value.as_str()));
    match fault {
        Some(fault) => (QuickStatus::Routed, QuickAction::InjectFault { fault }),
        None => (QuickStatus::Unknown, QuickAction::TextSearch),
    }
}

fn view_action(answers: Option<&Map<String, Value>>) -> (QuickStatus, QuickAction) {
    let view = argument(
        map_choice(answer(answers, VIEW), &with_unspecified(&VIEWS)),
        map_noul(answer(answers, VIEW_STATED)),
    )
    .filter(|value| VIEWS.contains(&value.as_str()));
    match view {
        Some(view) => (QuickStatus::Routed, QuickAction::GoTo { view }),
        None => (QuickStatus::Unknown, QuickAction::TextSearch),
    }
}

fn argument(choice: ChoiceFill, stated: NoulFill) -> Option<String> {
    // A confident "not stated" drops the argument. A Choice that cleared
    // 0.80 still counts when the stated Noul is uncertain: that Noul is not
    // a second pass bar.
    if stated == NoulFill::No {
        return None;
    }
    match choice {
        ChoiceFill::Value(value) => Some(value),
        ChoiceFill::Unspecified | ChoiceFill::Unknown => None,
    }
}

fn answer<'a>(answers: Option<&'a Map<String, Value>>, id: &str) -> Option<&'a Value> {
    answers.and_then(|answers| answers.get(id))
}

enum ChoiceFill {
    Value(String),
    Unspecified,
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NoulFill {
    Yes,
    No,
    Unknown,
}

fn map_choice(answer: Option<&Value>, options: &[String]) -> ChoiceFill {
    let Some(answer) = answer else {
        return ChoiceFill::Unknown;
    };
    if answer.get("type").and_then(Value::as_str) != Some("choice") {
        return ChoiceFill::Unknown;
    }
    let Some(choice) = answer.get("choice").and_then(Value::as_str) else {
        return ChoiceFill::Unknown;
    };
    let Some(probabilities) = answer.get("probabilities").and_then(Value::as_object) else {
        return ChoiceFill::Unknown;
    };
    if probabilities.len() != options.len() {
        return ChoiceFill::Unknown;
    }
    let mut parsed = Vec::with_capacity(options.len());
    for option in options {
        let Some(probability) = probabilities.get(option).and_then(Value::as_f64) else {
            return ChoiceFill::Unknown;
        };
        if !unit(probability) {
            return ChoiceFill::Unknown;
        }
        parsed.push((option.as_str(), probability));
    }
    let sum: f64 = parsed.iter().map(|(_, probability)| *probability).sum();
    if (sum - 1.0).abs() > PROBABILITY_SUM_TOLERANCE {
        return ChoiceFill::Unknown;
    }
    let clearing: Vec<_> = parsed
        .iter()
        .filter(|(_, probability)| *probability >= SAME_PROBABILITY_THRESHOLD)
        .collect();
    if clearing.len() != 1 {
        return ChoiceFill::Unknown;
    }
    let (winner, winner_p) = clearing[0];
    if choice != *winner {
        return ChoiceFill::Unknown;
    }
    if parsed
        .iter()
        .any(|(name, probability)| *name != *winner && *probability >= *winner_p)
    {
        return ChoiceFill::Unknown;
    }
    if *winner == UNSPECIFIED {
        ChoiceFill::Unspecified
    } else {
        ChoiceFill::Value((*winner).to_owned())
    }
}

fn map_noul(answer: Option<&Value>) -> NoulFill {
    let Some(answer) = answer else {
        return NoulFill::Unknown;
    };
    if answer.get("type").and_then(Value::as_str) != Some("noul") {
        return NoulFill::Unknown;
    }
    let Some(p_yes) = answer.get("noul").and_then(Value::as_f64) else {
        return NoulFill::Unknown;
    };
    if let Some(probabilities) = answer.get("probabilities") {
        if !noul_probabilities_agree(probabilities, p_yes) {
            return NoulFill::Unknown;
        }
    }
    if !unit(p_yes) {
        return NoulFill::Unknown;
    }
    let yes_clears = p_yes >= SAME_PROBABILITY_THRESHOLD;
    let no_clears = (1.0 - p_yes) >= DIFFERENT_PROBABILITY_THRESHOLD;
    if yes_clears && no_clears {
        return NoulFill::Unknown;
    }
    if yes_clears {
        NoulFill::Yes
    } else if no_clears {
        NoulFill::No
    } else {
        NoulFill::Unknown
    }
}

fn noul_probabilities_agree(probabilities: &Value, p_yes: f64) -> bool {
    let Some(object) = probabilities.as_object() else {
        return false;
    };
    if object.len() != 2 {
        return false;
    }
    let Some(p_true) = object.get("true").and_then(Value::as_f64) else {
        return false;
    };
    let Some(p_false) = object.get("false").and_then(Value::as_f64) else {
        return false;
    };
    if !unit(p_true) || !unit(p_false) {
        return false;
    }
    let yes_clears = p_true >= SAME_PROBABILITY_THRESHOLD;
    let no_clears = p_false >= DIFFERENT_PROBABILITY_THRESHOLD;
    if yes_clears && no_clears {
        return false;
    }
    if (p_true - p_yes).abs() > PROBABILITY_SUM_TOLERANCE {
        return false;
    }
    (p_true + p_false - 1.0).abs() <= PROBABILITY_SUM_TOLERANCE
}

fn handler_options() -> Vec<String> {
    [
        "filter_inbox",
        "go_to_view",
        "inject_fault",
        "none",
        "open_verdict",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn with_unspecified(values: &[&str]) -> Vec<String> {
    let mut options: Vec<String> = values.iter().map(|value| (*value).to_owned()).collect();
    options.push(UNSPECIFIED.to_owned());
    options.sort();
    options
}

fn string_slice(values: &[String]) -> Vec<&str> {
    values.iter().map(String::as_str).collect()
}

fn merge_list(required: &[String], extra: &[String], limit: usize) -> Vec<String> {
    let mut out = Vec::new();
    for value in required.iter().chain(extra.iter()) {
        let Some(clean) = clean_token(value) else {
            continue;
        };
        if clean == UNSPECIFIED || out.iter().any(|existing| existing == &clean) {
            continue;
        }
        out.push(clean);
        if out.len() == limit {
            break;
        }
    }
    out.sort();
    out
}

fn clean_token(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 80 {
        return None;
    }
    if !trimmed.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | ':')
    }) {
        return None;
    }
    Some(trimmed.to_owned())
}

fn unit(probability: f64) -> bool {
    probability.is_finite() && (0.0..=1.0).contains(&probability)
}

fn strip_explanation(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, _| key != "explanation");
            for child in map.values_mut() {
                strip_explanation(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_explanation(item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn sort_objects(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = BTreeMap::new();
            for (key, child) in map {
                sorted.insert(key, sort_objects(child));
            }
            let mut object = Map::new();
            for (key, child) in sorted {
                object.insert(key, child);
            }
            Value::Object(object)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_objects).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogue() -> Candidates {
        Candidates::catalogue()
    }

    fn choice_body(answers: Value) -> String {
        serde_json::json!({
            "model": "jev-test",
            "answers": answers
        })
        .to_string()
    }

    fn choice(label: &str, options: &[String], winner: f64) -> Value {
        let rest = options.len().saturating_sub(1);
        let each = if rest == 0 {
            0.0
        } else {
            (1.0 - winner) / rest as f64
        };
        let mut probabilities = Map::new();
        for option in options {
            let probability = if option == label { winner } else { each };
            probabilities.insert(option.clone(), serde_json::json!(probability));
        }
        serde_json::json!({
            "type": "choice",
            "choice": label,
            "probabilities": probabilities
        })
    }

    fn noul(probability: f64) -> Value {
        serde_json::json!({ "type": "noul", "noul": probability })
    }

    fn stated_yes() -> Value {
        noul(0.91)
    }

    fn stated_no() -> Value {
        noul(0.05)
    }

    fn filter_answers(
        verdict: bool,
        amount: bool,
        agent: Option<&str>,
        spec: Option<&str>,
    ) -> Value {
        let candidates = catalogue();
        let mut answers = Map::new();
        answers.insert(
            HANDLER.to_owned(),
            choice("filter_inbox", &handler_options(), 0.92),
        );
        answers.insert(
            VERDICT.to_owned(),
            choice(
                if verdict { "Violate" } else { UNSPECIFIED },
                &with_unspecified(&VERDICT_OPTIONS),
                0.90,
            ),
        );
        answers.insert(
            VERDICT_STATED.to_owned(),
            if verdict { stated_yes() } else { stated_no() },
        );
        answers.insert(
            AMOUNT.to_owned(),
            choice(
                if amount { "over-1000" } else { UNSPECIFIED },
                &with_unspecified(&["over-1000"]),
                0.88,
            ),
        );
        answers.insert(
            AMOUNT_STATED.to_owned(),
            if amount { stated_yes() } else { stated_no() },
        );
        let agent_label = agent.unwrap_or(UNSPECIFIED);
        answers.insert(
            AGENT.to_owned(),
            choice(
                agent_label,
                &with_unspecified(&string_slice(&candidates.agents)),
                0.86,
            ),
        );
        answers.insert(
            AGENT_STATED.to_owned(),
            if agent.is_some() {
                stated_yes()
            } else {
                stated_no()
            },
        );
        let spec_label = spec.unwrap_or(UNSPECIFIED);
        answers.insert(
            SPEC.to_owned(),
            choice(
                spec_label,
                &with_unspecified(&string_slice(&candidates.specs)),
                0.87,
            ),
        );
        answers.insert(
            SPEC_STATED.to_owned(),
            if spec.is_some() {
                stated_yes()
            } else {
                stated_no()
            },
        );
        for (id, options) in [
            (FAULT, with_unspecified(&FAULTS)),
            (VIEW, with_unspecified(&VIEWS)),
            (VERDICT_ID, with_unspecified(&[])),
        ] {
            answers.insert(id.to_owned(), choice(UNSPECIFIED, &options, 0.95));
        }
        answers.insert(FAULT_STATED.to_owned(), stated_no());
        answers.insert(VIEW_STATED.to_owned(), stated_no());
        answers.insert(VERDICT_ID_STATED.to_owned(), stated_no());
        Value::Object(answers)
    }

    #[test]
    fn request_asks_closed_sets_and_for_no_explanation() {
        let request = quick_request("violations over $1,000 by agent-2", &catalogue());
        let value: Value = serde_json::from_str(&request).expect("request JSON");
        assert_eq!(value["model"], "jev-latest");
        let questions = value["questions"].as_object().expect("questions");
        assert_eq!(questions.len(), 15);
        assert_eq!(questions[HANDLER]["type"], "choice");
        assert!(questions[HANDLER]["criteria"]
            .as_object()
            .expect("criteria")
            .contains_key("none"));
        assert_eq!(questions[VERDICT_STATED]["type"], "noul");
        assert!(value.get("explanation").is_none());
        assert!(!request.to_ascii_lowercase().contains("explain"));
        assert!(!request.to_ascii_lowercase().contains("fraud"));
        assert!(!request.contains("TYPESAFE_API_KEY"));
        assert!(!request.contains("Bearer"));
        assert_eq!(value["state"]["agents"][1], "agent-2");
    }

    #[test]
    fn merge_keeps_catalogue_and_drops_free_text() {
        let extra = Candidates {
            agents: vec![
                "agent.payer".to_owned(),
                "drop this".to_owned(),
                "agent-2".to_owned(),
            ],
            specs: vec!["ledger.custom.spec".to_owned()],
            verdicts: vec!["not a uuid".to_owned(), "v-1".to_owned()],
        };
        let merged = catalogue().merge(&extra);
        assert!(merged.agents.iter().any(|agent| agent == "agent-2"));
        assert!(merged.agents.iter().any(|agent| agent == "agent.payer"));
        assert!(!merged.agents.iter().any(|agent| agent.contains(' ')));
        assert!(merged.specs.iter().any(|spec| spec == "ledger.custom.spec"));
        assert_eq!(merged.verdicts, vec!["v-1".to_owned()]);
    }

    #[test]
    fn violations_phrase_shape_maps_three_chips() {
        let body = choice_body(filter_answers(true, true, Some("agent-2"), None));
        let reply =
            map_quick_exchange("violations over $1,000 by agent-2", &catalogue(), Ok(&body))
                .expect("phrase");
        assert_eq!(reply.status, QuickStatus::Routed);
        assert_eq!(
            reply.action,
            QuickAction::FilterInbox {
                verdict: Some("Violate".to_owned()),
                agent: Some("agent-2".to_owned()),
                spec: None,
                amount: Some("over-1000".to_owned()),
            }
        );
    }

    #[test]
    fn handler_under_threshold_is_text_search() {
        let mut answers = filter_answers(true, true, Some("agent-2"), None);
        answers[HANDLER] = choice("filter_inbox", &handler_options(), 0.79);
        let body = choice_body(answers);
        let reply =
            map_quick_exchange("violations over $1,000 by agent-2", &catalogue(), Ok(&body))
                .expect("phrase");
        assert_eq!(reply.status, QuickStatus::Unknown);
        assert_eq!(reply.action, QuickAction::TextSearch);
    }

    #[test]
    fn choice_that_disagrees_with_probabilities_is_unknown() {
        let mut answers = filter_answers(true, true, Some("agent-2"), None);
        let mut flipped = choice("none", &handler_options(), 0.92);
        flipped["choice"] = Value::String("filter_inbox".to_owned());
        answers[HANDLER] = flipped;
        let body = choice_body(answers);
        let reply =
            map_quick_exchange("violations over $1,000 by agent-2", &catalogue(), Ok(&body))
                .expect("phrase");
        assert_eq!(reply.status, QuickStatus::Unknown);
        assert_eq!(reply.action, QuickAction::TextSearch);
    }

    #[test]
    fn uncertain_stated_noul_keeps_a_choice_at_the_threshold() {
        let mut answers = filter_answers(true, true, Some("agent-2"), None);
        answers[VERDICT_STATED] = noul(0.32);
        answers[AMOUNT_STATED] = noul(0.75);
        answers[AGENT_STATED] = noul(0.63);
        let body = choice_body(answers);
        let reply =
            map_quick_exchange("violations over $1,000 by agent-2", &catalogue(), Ok(&body))
                .expect("phrase");
        assert_eq!(
            reply.action,
            QuickAction::FilterInbox {
                verdict: Some("Violate".to_owned()),
                agent: Some("agent-2".to_owned()),
                spec: None,
                amount: Some("over-1000".to_owned()),
            }
        );
    }

    #[test]
    fn stated_no_omits_the_argument() {
        let mut answers = filter_answers(true, true, Some("agent-2"), None);
        answers[AGENT_STATED] = stated_no();
        let body = choice_body(answers);
        let reply =
            map_quick_exchange("violations over $1,000 by agent-2", &catalogue(), Ok(&body))
                .expect("phrase");
        let QuickAction::FilterInbox { agent, .. } = reply.action else {
            panic!("expected a filter");
        };
        assert_eq!(agent, None);
    }

    #[test]
    fn none_is_unsupported_text_search() {
        let candidates = catalogue();
        let mut answers = filter_answers(false, false, None, None);
        answers[HANDLER] = choice("none", &handler_options(), 0.93);
        let body = choice_body(answers);
        let reply = map_quick_exchange("what is the weather today", &candidates, Ok(&body))
            .expect("phrase");
        assert_eq!(reply.status, QuickStatus::Unsupported);
        assert_eq!(reply.action, QuickAction::TextSearch);
    }

    #[test]
    fn fault_route_names_the_fault_and_does_not_build_a_verdict() {
        let mut answers = filter_answers(false, false, None, None);
        answers[HANDLER] = choice("inject_fault", &handler_options(), 0.94);
        answers[FAULT] = choice("double-pay", &with_unspecified(&FAULTS), 0.91);
        answers[FAULT_STATED] = stated_yes();
        let body = choice_body(answers);
        let reply = map_quick_exchange("inject a double-pay fault", &catalogue(), Ok(&body))
            .expect("phrase");
        assert_eq!(reply.status, QuickStatus::Routed);
        assert_eq!(
            reply.action,
            QuickAction::InjectFault {
                fault: "double-pay".to_owned()
            }
        );
        let rendered = serde_json::to_string(&reply.action).expect("action JSON");
        assert!(!rendered.contains("fraud"));
        assert!(!rendered.contains("verdict"));
        assert!(!rendered.contains("Violate"));
    }

    #[test]
    fn missing_verdict_id_does_not_invent_one() {
        let mut answers = filter_answers(false, false, None, None);
        answers[HANDLER] = choice("open_verdict", &handler_options(), 0.90);
        answers[VERDICT_ID] = choice(UNSPECIFIED, &with_unspecified(&[]), 0.97);
        answers[VERDICT_ID_STATED] = stated_no();
        let body = choice_body(answers);
        let reply = map_quick_exchange("open verdict not-on-the-stream", &catalogue(), Ok(&body))
            .expect("phrase");
        assert_eq!(reply.status, QuickStatus::Missing);
        assert_eq!(reply.action, QuickAction::MissingTarget);
    }

    #[test]
    fn verdict_id_outside_the_candidate_list_is_not_opened() {
        let mut candidates = catalogue();
        candidates.verdicts = vec!["v-real".to_owned()];
        let merged = catalogue().merge(&candidates);
        let mut answers = filter_answers(false, false, None, None);
        answers[HANDLER] = choice("open_verdict", &handler_options(), 0.90);
        let options = with_unspecified(&string_slice(&merged.verdicts));
        answers[VERDICT_ID] = choice("v-real", &options, 0.91);
        answers[VERDICT_ID_STATED] = stated_yes();
        let body = choice_body(answers);
        let reply =
            map_quick_exchange("open verdict v-real", &candidates, Ok(&body)).expect("phrase");
        assert_eq!(
            reply.action,
            QuickAction::OpenVerdict {
                id: "v-real".to_owned()
            }
        );

        let stray = Candidates {
            agents: Vec::new(),
            specs: Vec::new(),
            verdicts: vec!["v-real".to_owned()],
        };
        let mut answers = filter_answers(false, false, None, None);
        answers[HANDLER] = choice("open_verdict", &handler_options(), 0.90);
        let mut picked = choice("v-real", &with_unspecified(&["v-real"]), 0.91);
        picked["choice"] = Value::String("not-on-the-stream".to_owned());
        answers[VERDICT_ID] = picked;
        answers[VERDICT_ID_STATED] = stated_yes();
        let body = choice_body(answers);
        let reply = map_quick_exchange("open verdict not-on-the-stream", &stray, Ok(&body))
            .expect("phrase");
        assert_eq!(reply.status, QuickStatus::Missing);
        assert_eq!(reply.action, QuickAction::MissingTarget);
    }

    #[test]
    fn call_failures_are_unknown_text_search() {
        for failure in [
            JevCallFailure::Timeout,
            JevCallFailure::Transport,
            JevCallFailure::Http { status: 422 },
            JevCallFailure::Utf8,
        ] {
            let reply = map_quick_exchange(
                "violations over $1,000 by agent-2",
                &catalogue(),
                Err(failure),
            )
            .expect("phrase");
            assert_eq!(reply.status, QuickStatus::Unknown);
            assert_eq!(reply.action, QuickAction::TextSearch);
            assert!(reply.exchange.response.is_none());
            assert!(reply.exchange.failure.is_some());
        }
    }

    #[test]
    fn explanation_text_is_removed_from_the_stored_output() {
        let mut answers = filter_answers(true, false, None, None);
        answers[VERDICT]["explanation"] =
            Value::String("model text that must not be kept".to_owned());
        let body = choice_body(answers);
        let reply = map_quick_exchange("violations", &catalogue(), Ok(&body)).expect("phrase");
        let rendered = serde_json::to_string(&reply).expect("reply JSON");
        assert!(!rendered.contains("model text"));
        assert!(!rendered.contains("explanation"));
    }

    #[test]
    fn empty_phrase_is_rejected() {
        assert_eq!(
            map_quick_exchange("  ", &catalogue(), Ok("{}")).expect_err("empty"),
            QuickError::Empty
        );
    }

    #[test]
    fn unavailable_client_does_not_invent_a_route() {
        let reply = route_phrase(
            "violations over $1,000 by agent-2",
            &Candidates::none(),
            None,
        )
        .expect("phrase");
        assert_eq!(reply.status, QuickStatus::Unknown);
        assert_eq!(reply.action, QuickAction::TextSearch);
        assert_eq!(reply.exchange.failure, Some(QuickFailure::Unavailable));
        assert!(reply.exchange.response.is_none());
    }
}
