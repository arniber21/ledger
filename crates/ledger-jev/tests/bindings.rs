//! Replay the three stored binding calls. No socket is opened.

use std::fs;
use std::path::PathBuf;

use ledger_jev::{
    baseline_cases, baseline_report, binding_request, BindingEffect, BindingVerdict, GuardDecision,
    SystemOne,
};
use serde_json::Value;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/jev/bindings")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing fixture {}", path.display()))
}

#[test]
fn three_cases_replay_offline_and_report_cases() {
    let cases = baseline_cases();
    let mut responses = Vec::new();
    for case in &cases {
        let request = fixture(&format!("{}-request.json", case.id));
        let response = fixture(&format!("{}-response.json", case.id));
        assert_eq!(binding_request(&case.left, &case.right), request);
        for raw in [&request, &response] {
            assert!(!raw.contains("TYPESAFE_API_KEY"));
            assert!(!raw.contains("GEMINI_API_KEY"));
            assert!(!raw.contains("Bearer"));
            assert!(!raw.contains("explanation"));
            assert!(!raw.to_ascii_lowercase().contains("explain"));
        }
        let value: Value = serde_json::from_str(&response).expect("response json");
        assert!(value["model"].as_str().unwrap_or("").starts_with("jev"));
        responses.push(response);
    }

    let report = baseline_report([
        responses[0].as_str(),
        responses[1].as_str(),
        responses[2].as_str(),
    ]);
    assert_eq!(report.cases().len(), 3);
    let json = report.to_json();
    assert!(!json.contains("accuracy"));
    assert!(!json.contains("fraud"));

    let alias = &report.cases()[0];
    assert_eq!(alias.id, "abbreviated-vendor");
    assert_eq!(alias.exact, BindingVerdict::Different);
    assert_eq!(alias.fuzzy, BindingVerdict::Different);
    assert_eq!(alias.jev, BindingVerdict::Same);
    assert!(alias.jev_changed_finding);
    assert!(alias.resolver_called_jev);
    assert!(alias.effect.merged());
    assert_eq!(alias.effect.monitor_keys(), ["invoice:INV-1044"]);

    let rent = &report.cases()[1];
    assert_eq!(rent.id, "recurring-rent");
    assert_eq!(rent.exact, BindingVerdict::Different);
    assert_eq!(rent.fuzzy, BindingVerdict::Same);
    assert!(rent.fuzzy_would_merge);
    assert_eq!(rent.guard, GuardDecision::Different);
    assert_eq!(rent.jev, BindingVerdict::Different);
    assert_eq!(rent.resolver, BindingVerdict::Different);
    assert!(!rent.resolver_called_jev);
    assert!(!rent.effect.merged());
    assert_eq!(
        rent.effect.monitor_keys(),
        ["bill-operating-00003", "bill-operating-00018"]
    );
    assert!(matches!(rent.effect, BindingEffect::Distinct { .. }));

    let other = &report.cases()[2];
    assert_eq!(other.id, "different-bills");
    assert_eq!(other.exact, BindingVerdict::Different);
    assert_eq!(other.fuzzy, BindingVerdict::Different);
    assert_eq!(other.jev, BindingVerdict::Different);
    assert_eq!(other.resolver, BindingVerdict::Different);
    assert!(!other.resolver_called_jev);
    assert!(!other.jev_changed_finding);
    assert!(!other.effect.merged());

    assert!(report.cases().iter().any(|case| case.jev_changed_finding));
}

/// Writes `fixtures/jev/bindings/` from three live System One calls.
///
/// `just test` does not run this. Replay covers the saved bodies.
#[test]
#[ignore]
fn record_live_baseline_calls() {
    let key = std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY");
    let client = ledger_jev::SystemOneClient::new(key.clone()).expect("client");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jev/bindings");
    fs::create_dir_all(&root).expect("fixtures dir");
    for case in baseline_cases() {
        let request = binding_request(&case.left, &case.right);
        assert!(!request.contains(&key));
        let response = client.evaluate(&request).expect("system one call");
        assert!(!response.contains(&key));
        assert!(!response.contains("Bearer"));
        let response = strip_explanation(&response);
        fs::write(root.join(format!("{}-request.json", case.id)), &request).expect("request");
        fs::write(root.join(format!("{}-response.json", case.id)), &response).expect("response");
    }
}

fn strip_explanation(body: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(body) else {
        return body.to_owned();
    };
    fn walk(value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.retain(|key, _| key != "explanation");
                for child in map.values_mut() {
                    walk(child);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item);
                }
            }
            _ => {}
        }
    }
    walk(&mut value);
    serde_json::to_string(&value).unwrap_or_else(|_| body.to_owned())
}
