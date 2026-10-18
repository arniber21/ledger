//! A double-pay fault stored through ingest becomes a Violate on the SSE stream.

use std::time::Duration;

use chrono::{DateTime, Utc};
use ledger_api::{serve, Engine, StreamMessage};
use ledger_ingest::{persist, CapturedSpan};
use ledger_store::{database_url, Store, Uuid};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;

/// Each test gets its own scratch database on the local Timescale server. A
/// stack running against the shared database would otherwise step these events
/// first and insert the same deterministic verdict, leaving the poll empty.
async fn test_store() -> (Store, String) {
    Store::create_scratch(&database_url())
        .await
        .expect("create scratch database on local timescale (just db-up)")
}

async fn cleanup(store: Store, name: String) {
    store
        .drop_scratch(&database_url(), &name)
        .await
        .expect("drop scratch database");
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
    let (store, scratch) = test_store().await;
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

    let store_for_cleanup = store.clone();
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
    drop(server);
    cleanup(store_for_cleanup, scratch).await;
}

#[tokio::test]
async fn unresolved_order_after_three_ticks_violates() {
    let (store, scratch) = test_store().await;
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
    cleanup(store, scratch).await;
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

fn at(base: DateTime<Utc>, ms: i64) -> DateTime<Utc> {
    base + chrono::Duration::milliseconds(ms)
}

/// Spans shaped like the live desk's: the approval is exported in an earlier
/// batch than the payment, and its `approval.id` is a business id.
async fn persist_live_approval(store: &Store, bill: &str, approval: &str, now: DateTime<Utc>) {
    persist(
        store,
        &[span(
            "agent.approve",
            1,
            now,
            &[
                ("agent.id", "approver"),
                ("bill.id", bill),
                ("approval.id", approval),
            ],
        )],
    )
    .await
    .expect("persist approval");
}

async fn poll_for(
    engine: &mut Engine,
    wanted: &[(&str, &str)],
    bill: &str,
) -> Vec<ledger_api::StreamVerdict> {
    let mut seen: Vec<ledger_api::StreamVerdict> = Vec::new();
    for _ in 0..30 {
        seen.extend(
            engine
                .poll_once()
                .await
                .expect("poll")
                .into_iter()
                .filter(|frame| frame.binding == bill),
        );
        if wanted.iter().all(|(spec, outcome)| {
            seen.iter()
                .any(|frame| frame.spec_id == *spec && frame.outcome == *outcome)
        }) {
            return seen;
        }
    }
    let summary: Vec<String> = seen
        .iter()
        .map(|frame| {
            format!(
                "{} {} {:?}",
                frame.spec_id, frame.outcome, frame.reason_code
            )
        })
        .collect();
    panic!("wanted {wanted:?} for {bill}, saw {summary:?}");
}

#[tokio::test]
async fn live_payment_with_settlement_and_credit_satisfies_absence_and_precedence() {
    let (store, scratch) = test_store().await;
    let bill = format!("company-001/bill-{}", Uuid::new_v4());
    let approval = format!("approval-{bill}");
    let (pay, settled, credit) = (
        Uuid::new_v4().to_string(),
        Uuid::new_v4().to_string(),
        Uuid::new_v4().to_string(),
    );
    let now = Utc::now() - chrono::Duration::seconds(30);
    persist_live_approval(&store, &bill, &approval, now).await;
    persist(
        &store,
        &[
            span(
                "agent.pay_bill",
                2,
                at(now, 5_000),
                &[
                    ("event.id", pay.as_str()),
                    ("agent.id", "payer"),
                    ("bill.id", bill.as_str()),
                    ("amount", "500.00"),
                    ("link.approval_id", approval.as_str()),
                ],
            ),
            span(
                "ledger.exchange.settled",
                3,
                at(now, 5_001),
                &[
                    ("event.id", settled.as_str()),
                    ("agent.id", "payer"),
                    ("bill.id", bill.as_str()),
                    ("link.pay_id", pay.as_str()),
                ],
            ),
            span(
                "ledger.exchange.credit",
                4,
                at(now, 5_002),
                &[
                    ("event.id", credit.as_str()),
                    ("agent.id", "payer"),
                    ("bill.id", bill.as_str()),
                    ("link.settled_id", settled.as_str()),
                ],
            ),
        ],
    )
    .await
    .expect("persist payment");

    let (tx, _rx) = broadcast::channel(16);
    let mut engine = Engine::new(store.clone(), tx).expect("engine");
    poll_for(
        &mut engine,
        &[
            ("ledger.bill.no-second-payment", "Satisfy"),
            ("ledger.bill.approval-before-payment", "Satisfy"),
        ],
        &bill,
    )
    .await;
    cleanup(store, scratch).await;
}

#[tokio::test]
async fn live_double_pay_is_a_violate_with_a_counterexample_timeline() {
    let (store, scratch) = test_store().await;
    let bill = format!("company-001/bill-{}", Uuid::new_v4());
    let approval = format!("approval-{bill}");
    let (pay, settled, again) = (
        Uuid::new_v4().to_string(),
        Uuid::new_v4().to_string(),
        Uuid::new_v4().to_string(),
    );
    let now = Utc::now() - chrono::Duration::seconds(30);
    persist_live_approval(&store, &bill, &approval, now).await;
    persist(
        &store,
        &[
            span(
                "agent.pay_bill",
                2,
                at(now, 5_000),
                &[
                    ("event.id", pay.as_str()),
                    ("agent.id", "payer"),
                    ("bill.id", bill.as_str()),
                    ("amount", "500.00"),
                    ("history.source", "books"),
                    ("link.approval_id", approval.as_str()),
                ],
            ),
            span(
                "ledger.exchange.settled",
                3,
                at(now, 5_001),
                &[
                    ("event.id", settled.as_str()),
                    ("agent.id", "payer"),
                    ("bill.id", bill.as_str()),
                    ("link.pay_id", pay.as_str()),
                    ("history.source", "books"),
                ],
            ),
            span(
                "agent.pay_bill",
                4,
                at(now, 5_002),
                &[
                    ("event.id", again.as_str()),
                    ("agent.id", "payer"),
                    ("bill.id", bill.as_str()),
                    ("amount", "500.00"),
                    ("fault.name", "double-pay"),
                    ("link.approval_id", approval.as_str()),
                    ("link.settled_id", settled.as_str()),
                ],
            ),
        ],
    )
    .await
    .expect("persist double pay");

    let (tx, _rx) = broadcast::channel(16);
    let mut engine = Engine::new(store.clone(), tx).expect("engine");
    let frames = poll_for(
        &mut engine,
        &[("ledger.bill.no-second-payment", "Violate")],
        &bill,
    )
    .await;
    let frame = frames
        .iter()
        .find(|frame| frame.outcome == "Violate")
        .expect("violate");
    assert_eq!(frame.reason_code.as_deref(), Some("second-payment"));
    let roles: Vec<&str> = frame
        .timeline
        .iter()
        .map(|entry| entry.role.as_str())
        .collect();
    assert!(roles.contains(&"second-payment"), "{roles:?}");
    assert!(roles.contains(&"settlement"), "{roles:?}");
    cleanup(store, scratch).await;
}
