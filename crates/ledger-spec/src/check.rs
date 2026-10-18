//! Jev check: does the draft express the same property as the policy sentence?
//!
//! The Choice is `same` or `different`. Mapping goes through
//! [`ledger_jev::map_choice_body`] and the fixed 0.80 thresholds. This module
//! does not set its own thresholds and does not ask for explanation text.

use sentinel_compile::catalogue::catalogue;
use serde_json::Value;

use crate::draft::{parameters_json, sort_objects, PatternDraft};

/// Question id for the draft-match Choice.
///
/// The id is for this crate. The model sees [`match_request`]'s instructions
/// and criteria, not this string.
pub const DRAFT_MATCH_QUESTION: &str = "draft_matches_policy";

/// Body of `POST https://api.typesafe.ai/v1/systemone` for one draft.
///
/// The body has no API key.
#[must_use]
pub fn match_request(sentence: &str, draft: &PatternDraft) -> String {
    // Same sorted object keys as [`crate::draft::draft_request`].
    let value = sort_objects(match_request_value(sentence, draft));
    serde_json::to_string(&value).expect("match request is finite JSON")
}

fn match_request_value(sentence: &str, draft: &PatternDraft) -> Value {
    let summary = catalogue()
        .iter()
        .find(|item| item.id.as_str() == draft.pattern_id())
        .map(|item| item.summary)
        .unwrap_or("This pattern_id is not in the catalogue.");
    serde_json::json!({
        "state": {
            "policy": sentence,
            "pattern_id": draft.pattern_id(),
            "parameters": parameters_json(draft),
            "catalogue_summary": summary
        },
        "model": "jev-latest",
        "questions": {
            DRAFT_MATCH_QUESTION: {
                "type": "choice",
                "instructions": "Does `pattern_id` with `parameters` express the same property as `policy`, or a different property? `catalogue_summary` is what that pattern_id means.",
                "criteria": {
                    "same": {
                        "what": "The draft expresses the same property as the policy.",
                        "still_same": "Wording can differ when the pattern, the events, and the binding still state the policy.",
                        "not_for": "Do not pick this when the draft names a different property, event, or binding than the policy."
                    },
                    "different": {
                        "what": "The draft expresses a different property from the policy.",
                        "still_different": "A different pattern, event, or binding means it is not that policy.",
                        "not_for": "Do not pick this for wording alone when the property is the same."
                    }
                }
            }
        }
    })
}
