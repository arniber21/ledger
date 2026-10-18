//! A double-pay fault stored through ingest becomes a Violate on the SSE stream.

use std::time::Duration;

use chrono::{DateTime, Utc};
use ledger_api::{serve, Engine, StreamMessage};
use ledger_ingest::{persist, CapturedSpan};
use ledger_store::{database_url, Store, Uuid};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;

/// Both tests step every recent row. A second engine that inserts the same
/// verdict first leaves this poll with nothing to publish.
static ENGINE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn test_store() -> Store {
    let store = Store::connect(&database_url())
        .await
        .expect("connect to local timescale (just db-up)");
    store.migrate().await.expect("apply migrations");
    store
}

fn span(name: &'static str, id: u64, at: DateTime<Utc>, fields: &[(&str, &str)]) -> CapturedSpan {
    CapturedSpan::new(
        name,
        id,
        None,
        at,
        fields
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
    )
}

#[tokio::test]
async fn double_pay_fault_is_a_violate_on_the_stream_with_a_timeline() {
    let _guard = ENGINE.lock().await;
    let store = test_store().await;
    let bill = format!("bill-{}", Uuid::new_v4());
    let agent = format!("payer-{}", Uuid::new_v4());
    let pay = Uuid::new_v4().to_string();
    let settled = Uuid::new_v4().to_string();
    let again = Uuid::new_v4().to_string();
    let now = Utc::now();
    let spans = [
        span(
            "agent.pay_bill",
            1,
            now,
            &[
                ("event.id", pay.as_str()),
                ("agent.id", agent.as_str()),
                ("bill.id", bill.as_str()),
                ("amount", "12.50"),
            ],
        ),
        span(
            "ledger.exchange.settled",
            2,
            now + chrono::Duration::milliseconds(1),
            &[
                ("event.id", settled.as_str()),
                ("agent.id", agent.as_str()),
                ("bill.id", bill.as_str()),
                ("link.pay_id", pay.as_str()),
            ],
        ),
        span(
            "agent.pay_bill",
            3,
            now + chrono::Duration::milliseconds(2),
            &[
                ("event.id", again.as_str()),
                ("agent.id", agent.as_str()),
                ("bill.id", bill.as_str()),
                ("fault.name", "double-pay"),
                ("link.settled_id", settled.as_str()),
                ("amount", "12.50"),
            ],
        ),
    ];
    persist(&store, &spans).await.expect("persist fault");

    let (tx, _rx) = broadcast::channel(16);
    let mut engine = Engine::new(store.clone(), tx).expect("engine");
    let mut found = None;
    let mut seen = Vec::new();
    for _ in 0..30 {
        let published = engine.poll_once().await.expect("poll");
        for frame in &published {
            if frame.spec_id == "ledger.bill.no-second-payment" && frame.evidence.contains(&again) {
                seen.push(format!(
                    "{} {}",
                    frame.outcome,
                    frame.reason_code.as_deref().unwrap_or("")
                ));
            }
        }
        if let Some(frame) = published.into_iter().find(|frame| {
            frame.spec_id == "ledger.bill.no-second-payment"
                && frame.outcome == "Violate"
                && frame.evidence.contains(&again)
        }) {
            found = Some(frame);
            break;
        }
    }
    let frame = found.unwrap_or_else(|| panic!("absence violate, saw {seen:?}"));
    assert_eq!(frame.title, "needs review");
    assert_eq!(frame.reason_code.as_deref(), Some("second-payment"));
    assert!(frame.timeline.len() >= 2, "{:?}", frame.timeline);
    assert!(frame
        .timeline
        .windows(2)
        .all(|pair| pair[0].observed_at <= pair[1].observed_at));
    assert!(frame
        .timeline
        .iter()
        .any(|entry| entry.role == "second-payment"));
    assert_eq!(frame.amount_cents, Some(1_250));
    let body = serde_json::to_string(&StreamMessage {
        kind: "verdict",
        verdict: frame.clone(),
    })
    .expect("json");
    assert!(!body.to_lowercase().contains("fraud"));

    let stored = store
        .fetch_verdict(frame.id)
        .await
        .expect("fetch")
        .expect("row");
    assert_eq!(stored.outcome.as_str(), "Violate");
    assert_eq!(stored.spec_id, "ledger.bill.no-second-payment");

    let server = serve(store, "127.0.0.1:0").await.expect("serve");
    let traces = read_http(server.addr, "/api/traces").await;
    assert!(
        traces.starts_with("HTTP/1.1 200"),
        "traces status: {traces}"
    );
    assert!(
        traces.contains("\"events\""),
        "traces response has no events array"
    );
    let payload = read_until(server.addr, &frame.id.to_string()).await;
    assert!(payload.contains("\"kind\":\"verdict\"") || payload.contains("\"kind\": \"verdict\""));
    assert!(
        payload.contains("\"outcome\":\"Violate\"") || payload.contains("\"outcome\": \"Violate\"")
    );
    assert!(payload.contains("needs review"));
    assert!(payload.contains("timeline"));
    assert!(payload.contains(&bill) || payload.contains(&again));
    assert!(!payload.to_lowercase().contains("fraud"));
}

#[tokio::test]
async fn unresolved_order_after_three_ticks_violates() {
    let _guard = ENGINE.lock().await;
    let store = test_store().await;
    let order = format!("order-{}", Uuid::new_v4());
    let now = Utc::now();
    let mut spans = vec![span(
        "exchange.submit_order",
        1,
        now,
        &[("order.id", order.as_str()), ("agent.id", "buyer")],
    )];
    for step in 1_u32..=3 {
        spans.push(span(
            "exchange.tick",
            u64::from(step + 1),
            now + chrono::Duration::milliseconds(i64::from(step)),
            &[("agent.id", "buyer")],
        ));
    }
    persist(&store, &spans).await.expect("persist order");

    let (tx, _rx) = broadcast::channel(8);
    let mut engine = Engine::new(store.clone(), tx).expect("engine");
    let mut found = None;
    for _ in 0..30 {
        let published = engine.poll_once().await.expect("poll");
        if let Some(frame) = published.into_iter().find(|frame| {
            frame.spec_id == "ledger.order.fill-or-cancel"
                && frame.binding == order
                && frame.outcome == "Violate"
        }) {
            found = Some(frame);
            break;
        }
    }
    let frame = found.expect("response violate");
    assert_eq!(frame.reason_code.as_deref(), Some("order-unresolved"));
    assert_eq!(frame.title, "needs review");
    assert!(frame.timeline.iter().any(|entry| entry.role == "order"));
    assert!(frame.timeline.iter().any(|entry| entry.role == "deadline"));
}

async fn read_http(addr: std::net::SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut buf = Vec::new();
    loop {
        let mut tmp = [0_u8; 8192];
        match stream.read(&mut tmp).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

async fn read_until(addr: std::net::SocketAddr, needle: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let request = format!(
        "GET /v1/verdicts/stream HTTP/1.1\r\nHost: {addr}\r\nAccept: text/event-stream\r\nConnection: keep-alive\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut buf = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        let text = String::from_utf8_lossy(&buf);
        if text.contains(needle) && text.contains("Violate") && text.contains("timeline") {
            return text.into_owned();
        }
        if tokio::time::Instant::now() > deadline {
            panic!("sse timed out: {text}");
        }
        let mut tmp = [0_u8; 8192];
        let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut tmp)).await;
        match read {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => continue,
            Ok(Ok(n)) => buf.extend_from_slice(&tmp[..n]),
        }
    }
}
