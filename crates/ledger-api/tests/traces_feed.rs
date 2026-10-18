//! The traces feed keeps every span on a trace, not only `exchange.submit_order`.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use ledger_api::serve;
use ledger_store::{database_url, Event, Store, Uuid};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

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

fn span(name: &str, span_id: &str, parent: Option<&str>, at: DateTime<Utc>, source: &str) -> Event {
    let mut payload = json!({
        "span": name,
        "span_id": span_id,
        "attributes": { "order.id": "order-1" },
        "adapted": false,
    });
    if let Some(parent) = parent {
        payload["parent_span_id"] = json!(parent);
    }
    let kind = if name == "agent.pay_bill" {
        "ledger.agent.pay_bill"
    } else {
        name
    };
    Event {
        id: Uuid::new_v4(),
        observed_at: at,
        source: source.to_owned(),
        kind: kind.to_owned(),
        data_class: "tenant".to_owned(),
        payload,
    }
}

#[tokio::test]
async fn feed_includes_match_fill_settle_and_agent_spans() {
    let (store, scratch) = test_store().await;
    let base = Utc::now();
    let trace = [
        span("agent.decide", "1", None, base, "agent.buyer"),
        span(
            "exchange.match",
            "3",
            Some("2"),
            base + Duration::milliseconds(1),
            "exchange",
        ),
        span(
            "exchange.fill",
            "4",
            Some("3"),
            base + Duration::milliseconds(2),
            "exchange",
        ),
        span(
            "exchange.settle",
            "5",
            Some("4"),
            base + Duration::milliseconds(3),
            "exchange",
        ),
        span(
            "agent.settle",
            "6",
            Some("1"),
            base + Duration::milliseconds(4),
            "agent.buyer",
        ),
        span(
            "agent.pay_bill",
            "7",
            Some("1"),
            base + Duration::milliseconds(5),
            "agent.payer",
        ),
    ];
    for event in &trace {
        store.insert_event(event).await.expect("insert trace span");
    }
    // Newest span on the trace. Two hundred newer rows would hide it without
    // the parent walk; one hundred ninety-nine fillers leave this submit inside
    // the page and the earlier spans outside it.
    store
        .insert_event(&span(
            "exchange.submit_order",
            "2",
            Some("1"),
            base + Duration::seconds(1),
            "exchange",
        ))
        .await
        .expect("insert submit");
    for index in 0..199 {
        store
            .insert_event(&span(
                "exchange.tick",
                &format!("tick-{index}"),
                None,
                base + Duration::seconds(2),
                "exchange",
            ))
            .await
            .expect("insert filler");
    }
    let window = format!("window-{}", Uuid::new_v4());
    for _ in 0..20 {
        store
            .insert_event(&Event {
                id: Uuid::new_v4(),
                observed_at: base + Duration::days(10),
                source: window.clone(),
                kind: "exchange.submit_order".to_owned(),
                data_class: "tenant".to_owned(),
                payload: json!({}),
            })
            .await
            .expect("insert window row");
    }

    let server = serve(store.clone(), "127.0.0.1:0").await.expect("serve");
    let raw = read_http(server.addr, "/api/traces").await;
    let store_for_cleanup = store.clone();
    drop(server);

    let outcome = check_feed(&raw, &window);
    cleanup(store_for_cleanup, scratch).await;
    outcome.expect("trace feed");
}

fn check_feed(raw: &str, window: &str) -> Result<(), String> {
    if !raw.starts_with("HTTP/1.1 200") {
        return Err(format!("traces status: {raw}"));
    }
    let body = raw
        .split("\r\n\r\n")
        .nth(1)
        .ok_or_else(|| format!("no body: {raw}"))?;
    let parsed: Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
    let events = parsed["events"]
        .as_array()
        .ok_or_else(|| format!("events missing: {body}"))?;
    let mut names = BTreeSet::new();
    for event in events {
        let source = event["source"].as_str().unwrap_or("");
        if source == window || source.starts_with("window-") {
            return Err(format!("window fixture leaked into the feed: {source}"));
        }
        if let Some(span) = event["payload"]["span"].as_str() {
            names.insert(span.to_owned());
        }
    }
    for name in [
        "agent.decide",
        "exchange.submit_order",
        "exchange.match",
        "exchange.fill",
        "exchange.settle",
        "agent.settle",
        "agent.pay_bill",
    ] {
        if !names.contains(name) {
            return Err(format!("missing {name}, saw {names:?}"));
        }
    }
    Ok(())
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
