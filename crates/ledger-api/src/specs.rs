//! `GET /api/specs` lists the built-in specs the verdict engine runs.
//!
//! The engine's `Session` steps all four on every event it reads, so each one
//! is armed whenever this API is up. Spec ids come from `ledger-core`. Specs
//! authored in the UI are not listed here: they live in the authoring stream.

use axum::routing::get;
use axum::{Json, Router};
use ledger_core::{SPEC_ABSENCE, SPEC_PRECEDENCE, SPEC_RESPONSE, SPEC_TAINT};
use serde_json::{json, Value};

/// One armed built-in spec.
struct Builtin {
    id: &'static str,
    pattern: &'static str,
    scope: &'static str,
    policy: &'static str,
}

const BUILTINS: [Builtin; 4] = [
    Builtin {
        id: SPEC_PRECEDENCE,
        pattern: "precedence",
        scope: "bill.id",
        policy: "A bill's approval comes before its payment.",
    },
    Builtin {
        id: SPEC_ABSENCE,
        pattern: "absence-after",
        scope: "bill.id",
        policy: "No second payment on a settled bill before a credit.",
    },
    Builtin {
        id: SPEC_RESPONSE,
        pattern: "response",
        scope: "order.id",
        policy: "An order is filled or cancelled before its deadline.",
    },
    Builtin {
        id: SPEC_TAINT,
        pattern: "precedence",
        scope: "bill.id",
        policy: "A payment whose memo instructs payment needs a prior approval.",
    },
];

/// Route for the armed built-in specs.
pub fn router() -> Router {
    Router::new().route("/api/specs", get(list))
}

async fn list() -> Json<Value> {
    Json(body())
}

fn body() -> Value {
    json!({
        "specs": BUILTINS.iter().map(|spec| json!({
            "id": spec.id,
            "pattern": spec.pattern,
            "scope": spec.scope,
            "policy": spec.policy,
            "status": "armed",
            "origin": "built-in",
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_shipped_specs_as_armed() {
        let value = body();
        let specs = value["specs"].as_array().expect("specs");
        assert_eq!(specs.len(), 4);
        assert!(specs.iter().all(|spec| spec["status"] == "armed"));
        assert!(specs
            .iter()
            .any(|spec| spec["id"] == "ledger.bill.approval-before-payment"));
        assert!(!value.to_string().to_ascii_lowercase().contains("fraud"));
    }
}
