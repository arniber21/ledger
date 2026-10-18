//! A Jev stand-in for the test API.
//!
//! It is used only when `LEDGER_TEST=1`. Same, Different, and Unknown are
//! binding Choice bodies. `timeout` is [`JevCallFailure::Timeout`], which
//! maps to Unknown and is not a pass. This module does not call System One
//! and does not write explanation text.

use serde_json::{json, Value};

use crate::{JevCallFailure, SAME_OBLIGATION_QUESTION};

/// One forced binding outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForcedOutcome {
    /// The records are one obligation.
    Same,
    /// The records are two obligations.
    Different,
    /// A score under the fixed threshold. Not a timeout.
    Unknown,
    /// The call exceeds the client limit.
    Timeout,
}

impl ForcedOutcome {
    /// Parses the test API's outcome name.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "Same" | "same" => Some(Self::Same),
            "Different" | "different" => Some(Self::Different),
            "Unknown" | "unknown" => Some(Self::Unknown),
            "timeout" | "Timeout" => Some(Self::Timeout),
            _ => None,
        }
    }

    /// The name the test API stores and returns.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Same => "Same",
            Self::Different => "Different",
            Self::Unknown => "Unknown",
            Self::Timeout => "timeout",
        }
    }

    /// Builds a Choice body, or fails with [`JevCallFailure::Timeout`].
    ///
    /// # Errors
    ///
    /// Returns [`JevCallFailure::Timeout`] for [`ForcedOutcome::Timeout`].
    pub fn apply(self, request: &str) -> Result<String, JevCallFailure> {
        match self {
            Self::Timeout => Err(JevCallFailure::Timeout),
            outcome => Ok(outcome.body(request)),
        }
    }

    fn body(self, request: &str) -> String {
        let (choice, same, different) = match self {
            Self::Same => ("same", 1.0, 0.0),
            Self::Different => ("different", 0.0, 1.0),
            Self::Unknown => ("same", 0.5, 0.5),
            Self::Timeout => ("same", 0.5, 0.5),
        };
        let answer = json!({
            "type": "choice",
            "choice": choice,
            "probabilities": { "same": same, "different": different }
        });
        let mut answers = serde_json::Map::new();
        for id in question_ids(request) {
            answers.insert(id, answer.clone());
        }
        json!({
            "model": "jev-test",
            "answers": Value::Object(answers)
        })
        .to_string()
    }
}

fn question_ids(request: &str) -> Vec<String> {
    let mut ids = Vec::new();
    if let Ok(value) = serde_json::from_str::<Value>(request) {
        if let Some(questions) = value.get("questions").and_then(Value::as_object) {
            ids.extend(questions.keys().cloned());
        }
    }
    if !ids.iter().any(|id| id == SAME_OBLIGATION_QUESTION) {
        ids.push(SAME_OBLIGATION_QUESTION.to_owned());
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{map_binding_body, map_binding_exchange};

    const REQUEST: &str = r#"{"questions":{"same_obligation":{"type":"choice"}}}"#;

    #[test]
    fn same_different_and_unknown_map_without_a_call() {
        let same = ForcedOutcome::Same.apply(REQUEST).expect("body");
        assert_eq!(map_binding_body(&same), crate::BindingVerdict::Same);
        let different = ForcedOutcome::Different.apply(REQUEST).expect("body");
        assert_eq!(
            map_binding_body(&different),
            crate::BindingVerdict::Different
        );
        let unknown = ForcedOutcome::Unknown.apply(REQUEST).expect("body");
        assert_eq!(map_binding_body(&unknown), crate::BindingVerdict::Unknown);
        for body in [same, different, unknown] {
            assert!(!body.contains("explanation"));
            assert!(!body.contains("fraud"));
        }
    }

    #[test]
    fn timeout_is_a_call_failure_mapped_to_unknown() {
        let error = ForcedOutcome::Timeout.apply(REQUEST).expect_err("timeout");
        assert_eq!(error, JevCallFailure::Timeout);
        assert_eq!(
            map_binding_exchange(Err(error)),
            crate::BindingVerdict::Unknown
        );
    }

    #[test]
    fn parse_rejects_other_names() {
        assert_eq!(
            ForcedOutcome::parse("timeout"),
            Some(ForcedOutcome::Timeout)
        );
        assert_eq!(ForcedOutcome::parse("Same"), Some(ForcedOutcome::Same));
        assert!(ForcedOutcome::parse("pass").is_none());
        assert!(ForcedOutcome::parse("fraud").is_none());
    }
}
