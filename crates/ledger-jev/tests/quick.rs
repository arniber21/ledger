//! Replay the stored Quick actions calls. No socket is opened.
//!
//! `record_live_quick_actions` is ignored. `just test` replays the saved bodies.

use std::fs;
use std::path::PathBuf;

use ledger_jev::{
    map_quick_exchange, quick_examples, quick_request, Candidates, QuickAction, QuickStatus,
};

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/jev/quick")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|_| panic!("missing fixture {}", path.display()))
}

#[test]
fn five_phrases_replay_offline() {
    let cases = [
        (
            "violations",
            QuickStatus::Routed,
            QuickAction::FilterInbox {
                verdict: Some("Violate".to_owned()),
                agent: Some("agent-2".to_owned()),
                spec: None,
                amount: Some("over-1000".to_owned()),
            },
        ),
        (
            "inconclusive",
            QuickStatus::Routed,
            QuickAction::FilterInbox {
                verdict: Some("Inconclusive".to_owned()),
                agent: None,
                spec: Some("ledger.bill.approval-before-payment".to_owned()),
                amount: None,
            },
        ),
        (
            "satisfied",
            QuickStatus::Routed,
            QuickAction::FilterInbox {
                verdict: Some("Satisfy".to_owned()),
                agent: Some("agent-1".to_owned()),
                spec: None,
                amount: None,
            },
        ),
        (
            "unsupported",
            QuickStatus::Unsupported,
            QuickAction::TextSearch,
        ),
        (
            "double-pay",
            QuickStatus::Routed,
            QuickAction::InjectFault {
                fault: "double-pay".to_owned(),
            },
        ),
    ];
    for example in quick_examples() {
        let request = fixture(&format!("{}-request.json", example.id));
        let response = fixture(&format!("{}-response.json", example.id));
        assert_eq!(
            quick_request(example.phrase, &Candidates::catalogue()),
            request,
            "{}",
            example.id
        );
        for raw in [&request, &response] {
            assert!(!raw.contains("TYPESAFE_API_KEY"));
            assert!(!raw.contains("GEMINI_API_KEY"));
            assert!(!raw.contains("Bearer"));
            assert!(!raw.contains("explanation"));
            assert!(!raw.to_ascii_lowercase().contains("fraud"));
        }
        let reply = map_quick_exchange(example.phrase, &Candidates::none(), Ok(&response))
            .unwrap_or_else(|_| panic!("phrase {}", example.id));
        let expected = cases
            .iter()
            .find(|(id, _, _)| *id == example.id)
            .unwrap_or_else(|| panic!("case {}", example.id));
        assert_eq!(reply.status, expected.1, "{}", example.id);
        assert_eq!(reply.action, expected.2, "{}", example.id);
        if example.id == "double-pay" {
            let rendered = serde_json::to_string(&reply.action).expect("action");
            assert!(!rendered.contains("verdict"));
        }
    }
}

/// Writes `fixtures/jev/quick/` from five live System One calls.
///
/// `just test` does not run this. Replay covers the saved bodies.
#[test]
#[ignore]
fn record_live_quick_actions() {
    let key = std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY");
    let client = ledger_jev::SystemOneClient::new(key.clone()).expect("client");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jev/quick");
    fs::create_dir_all(&root).expect("fixtures dir");
    let only = std::env::var("QUICK_RECORD").unwrap_or_default();
    for example in quick_examples() {
        if !only.is_empty() && !only.split(',').any(|id| id == example.id) {
            continue;
        }
        let reply = ledger_jev::route_phrase(example.phrase, &Candidates::none(), Some(&client))
            .unwrap_or_else(|_| panic!("route {}", example.id));
        let rendered = serde_json::to_string(&reply).expect("reply");
        assert!(!rendered.contains(&key));
        assert!(!rendered.contains("Bearer"));
        let request = quick_request(example.phrase, &Candidates::catalogue());
        let Some(response) = reply.exchange.response.as_ref() else {
            println!("{} failure {:?}", example.id, reply.exchange.failure);
            panic!("no body for {}", example.id);
        };
        if let Some(answers) = response.get("answers").and_then(|value| value.as_object()) {
            for (id, answer) in answers {
                let label = answer.get("choice").or_else(|| answer.get("noul"));
                println!("  {id} {label:?}");
            }
        }
        let response = serde_json::to_string(response).expect("response");
        assert!(!response.contains(&key));
        fs::write(root.join(format!("{}-request.json", example.id)), request).expect("request");
        fs::write(root.join(format!("{}-response.json", example.id)), response).expect("response");
        println!("{} {:?} {:?}", example.id, reply.status, reply.action);
    }
}
