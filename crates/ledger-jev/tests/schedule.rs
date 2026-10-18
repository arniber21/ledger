//! Replay the two stored schedule calls. No socket is opened.

use std::fs;
use std::path::PathBuf;

use ledger_jev::{
    schedule_cases, schedule_report, schedule_request, BindingVerdict, ScheduleFinding,
    ScheduleGuard, SystemOne, RECURRING_OR_DUPLICATE_QUESTION,
};
use serde_json::Value;

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/jev/schedule")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing fixture {}", path.display()))
}

#[test]
fn recurring_and_duplicate_replay_offline() {
    let cases = schedule_cases();
    let mut responses = Vec::new();
    for case in &cases {
        let request = fixture(&format!("{}-request.json", case.id));
        let response = fixture(&format!("{}-response.json", case.id));
        assert_eq!(schedule_request(&case.candidate, &case.history), request);
        for raw in [&request, &response] {
            assert!(!raw.contains("TYPESAFE_API_KEY"));
            assert!(!raw.contains("GEMINI_API_KEY"));
            assert!(!raw.contains("Bearer"));
            assert!(!raw.contains("explanation"));
            assert!(!raw.to_ascii_lowercase().contains("explain"));
            assert!(!raw.to_lowercase().contains("fraud"));
        }
        let value: Value = serde_json::from_str(&response).expect("response json");
        assert!(value["model"].as_str().unwrap_or("").starts_with("jev"));
        let answer = &value["answers"][RECURRING_OR_DUPLICATE_QUESTION];
        assert_eq!(answer["type"], "choice");
        responses.push(response);
    }

    let report = schedule_report([responses[0].as_str(), responses[1].as_str()]);
    assert_eq!(report.cases().len(), 2);
    let json = report.to_json();
    assert!(!json.contains("accuracy"));
    assert!(!json.to_lowercase().contains("fraud"));
    assert!(json.contains("needs review"));

    let rent = &report.cases()[0];
    assert_eq!(rent.id, "recurring-schedule");
    assert_eq!(rent.exact, BindingVerdict::Different);
    assert_eq!(rent.fuzzy, BindingVerdict::Same);
    assert!(rent.fuzzy_would_flag);
    assert_eq!(rent.guard, ScheduleGuard::Recurring);
    assert_eq!(rent.resolver, BindingVerdict::Different);
    assert_eq!(rent.finding, ScheduleFinding::Recurring);
    assert!(!rent.resolver_called_jev);
    assert_eq!(rent.jev, BindingVerdict::Different);
    assert!(!rent.jev_changed_finding);
    assert_eq!(
        rent.jev,
        ledger_jev::map_choice_body(&responses[0], RECURRING_OR_DUPLICATE_QUESTION)
    );

    let duplicate = &report.cases()[1];
    assert_eq!(duplicate.id, "duplicate-period");
    assert_eq!(duplicate.exact, BindingVerdict::Same);
    assert_eq!(duplicate.fuzzy, BindingVerdict::Same);
    assert!(duplicate.fuzzy_would_flag);
    assert_eq!(duplicate.guard, ScheduleGuard::Duplicate);
    assert_eq!(duplicate.resolver, BindingVerdict::Same);
    assert_eq!(duplicate.finding, ScheduleFinding::NeedsReview);
    assert_eq!(duplicate.finding.label(), "needs review");
    assert!(!duplicate.resolver_called_jev);
    assert_eq!(duplicate.jev, BindingVerdict::Same);
    assert_eq!(
        duplicate.jev,
        ledger_jev::map_choice_body(&responses[1], RECURRING_OR_DUPLICATE_QUESTION)
    );

    assert!(report
        .cases()
        .iter()
        .any(|case| case.finding == ScheduleFinding::Recurring));
    assert!(report
        .cases()
        .iter()
        .any(|case| case.finding == ScheduleFinding::NeedsReview));
}

/// Writes `fixtures/jev/schedule/` from two live System One calls.
///
/// `just test` does not run this. Replay covers the saved bodies.
#[test]
#[ignore]
fn record_live_schedule_calls() {
    let key = std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY");
    let client = ledger_jev::SystemOneClient::new(key.clone()).expect("client");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jev/schedule");
    fs::create_dir_all(&root).expect("fixtures dir");
    for case in schedule_cases() {
        let request = schedule_request(&case.candidate, &case.history);
        assert!(
            !request.contains(&key),
            "request must not contain the api key"
        );
        let response = client.evaluate(&request).expect("system one call");
        assert!(
            !response.contains(&key),
            "response must not contain the api key"
        );
        assert!(
            !response.contains("Bearer"),
            "response must not contain a bearer token"
        );
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
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
    walk(&mut value);
    serde_json::to_string(&value).unwrap_or_else(|_| body.to_owned())
}
