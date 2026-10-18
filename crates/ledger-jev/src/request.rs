//! System One request for one binding pair.
//!
//! The Choice criteria are the ones fixed for the first binding call. This
//! module does not ask for explanation text.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::obligation::{ref_state, ObligationRef};
use crate::SAME_OBLIGATION_QUESTION;

/// Body of `POST https://api.typesafe.ai/v1/systemone` for one pair.
///
/// The body has no API key. Object keys are sorted so the saved fixture does
/// not change when another crate turns on `serde_json`'s `preserve_order`.
#[must_use]
pub fn binding_request(left: &ObligationRef, right: &ObligationRef) -> String {
    let value = sort_objects(binding_request_value(left, right));
    serde_json::to_string(&value).expect("binding request is finite JSON")
}

fn binding_request_value(left: &ObligationRef, right: &ObligationRef) -> Value {
    serde_json::json!({
        "state": {
            "left": ref_state(left),
            "right": ref_state(right)
        },
        "model": "jev-latest",
        "questions": {
            SAME_OBLIGATION_QUESTION: {
                "type": "choice",
                "instructions": "Are `left` and `right` the same obligation (one bill the company owes), or two different obligations?",
                "criteria": {
                    "same": {
                        "what": "Both records refer to one obligation.",
                        "still_same": "Abbreviation, invoice punctuation, or memo wording can differ when vendor, amount, currency, and invoice identity still match.",
                        "not_for": "Do not pick this when vendor, amount, currency, or invoice identity disagrees."
                    },
                    "different": {
                        "what": "The records are two obligations.",
                        "still_different": "A different vendor, amount, currency, or invoice identity means they are not the same bill.",
                        "not_for": "Do not pick this for abbreviation or memo wording alone."
                    }
                }
            }
        }
    })
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
    use crate::binding_smoke_request;

    fn abbreviated() -> (ObligationRef, ObligationRef) {
        let left = ObligationRef::new("Acme Industrial Supply", "1250.00", "USD")
            .with_invoice_number("INV-1044")
            .with_memo("Net-30 parts order for the March plant shutdown");
        let right = ObligationRef::new("Acme Ind. Supply", "1250.00", "USD")
            .with_invoice_number("1044")
            .with_memo("Parts for March shutdown, terms net 30");
        (left, right)
    }

    #[test]
    fn request_reuses_the_fixed_choice_and_asks_for_no_explanation() {
        let (left, right) = abbreviated();
        let request = binding_request(&left, &right);
        let value: Value = serde_json::from_str(&request).expect("request JSON");
        let smoke: Value = serde_json::from_str(binding_smoke_request()).expect("smoke JSON");
        assert_eq!(value["model"], "jev-latest");
        assert_eq!(value["questions"], smoke["questions"]);
        assert_eq!(
            value["state"]["left"]["vendor_name"],
            "Acme Industrial Supply"
        );
        assert_eq!(value["state"]["right"]["invoice_number"], "1044");
        assert!(value["state"]["left"].get("bill_id").is_none());
        assert!(value.get("explanation").is_none());
        assert!(!request.to_ascii_lowercase().contains("explain"));
        assert!(!request.contains("TYPESAFE_API_KEY"));
        assert!(!request.contains("Bearer"));
    }
}
