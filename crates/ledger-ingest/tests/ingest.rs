//! Live spans land in Timescale and show up in the window query.

use std::collections::BTreeMap;

use chrono::Duration;
use ledger_ingest::{persist, IngestLayer};
use ledger_store::{BucketWidth, Store};
use tracing_subscriber::prelude::*;
use tracing_subscriber::Registry;

async fn test_store() -> Store {
    let store = Store::connect_test()
        .await
        .expect("connect to ledger_test (just db-up)");
    store
}

#[tokio::test]
async fn live_spans_round_trip_and_window_counts_them() {
    let store = test_store().await;
    let agent = format!("payer-{}", ledger_store::Uuid::new_v4());
    let (layer, receiver) = IngestLayer::pair();
    tracing::subscriber::with_default(Registry::default().with(layer), || {
        let decide = tracing::info_span!(
            "agent.decide",
            agent.id = agent.as_str(),
            decision.kind = "pay",
            bill.id = "bill-1"
        );
        let _decide = decide.enter();
        let pay = tracing::info_span!(
            "agent.pay_bill",
            agent.id = agent.as_str(),
            bill.id = "bill-1",
            payment.id = "pay-1",
            link.approval_id = "apr-1"
        );
        drop(pay.enter());
        let _submit =
            tracing::info_span!("exchange.submit_order", agent.id = "buyer", order.id = "7")
                .entered();
        let _matched = tracing::info_span!("exchange.match", order.id = "7").entered();
        let _filled = tracing::info_span!("exchange.fill", fill.id = "1", order.id = "7").entered();
        let _settled = tracing::info_span!("exchange.settle", fill.id = "1").entered();
    });

    let spans: Vec<_> = receiver.try_iter().collect();
    assert_eq!(spans.len(), 6);

    let stored = persist(&store, &spans).await.expect("persist spans");
    assert_eq!(stored.len(), 6);
    for event in &stored {
        let loaded = store
            .fetch_event(event.id)
            .await
            .expect("fetch event")
            .expect("stored event");
        assert_eq!(loaded, *event);
    }

    let pay = stored
        .iter()
        .find(|event| event.payload["span"] == "agent.pay_bill")
        .expect("pay event");
    assert_eq!(pay.kind, "ledger.agent.pay_bill");
    assert_eq!(pay.source, format!("agent.{agent}"));
    assert_eq!(pay.data_class, "tenant");
    assert_eq!(pay.payload["adapted"], true);
    assert_eq!(pay.payload["evidence"], "inconclusive");
    assert_eq!(
        pay.payload["causal_links"][0]["attribute"],
        "link.approval_id"
    );

    let decide = stored
        .iter()
        .find(|event| event.kind == "agent.decide")
        .expect("decide event");
    assert_eq!(decide.source, format!("agent.{agent}"));
    assert_eq!(decide.payload["adapted"], false);

    let submit = stored
        .iter()
        .find(|event| event.kind == "exchange.submit_order")
        .expect("submit event");
    assert_eq!(submit.source, "exchange");
    assert_eq!(submit.payload["parent_span_id"], decide.payload["span_id"]);
    assert_eq!(pay.payload["parent_span_id"], decide.payload["span_id"]);
    for name in ["exchange.match", "exchange.fill", "exchange.settle"] {
        let event = stored
            .iter()
            .find(|event| event.kind == name)
            .unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(event.source, "exchange");
        assert!(event.payload.get("parent_span_id").is_some());
    }

    let source = format!("agent.{agent}");
    let start = stored
        .iter()
        .filter(|event| event.source == source)
        .map(|event| event.observed_at)
        .min()
        .expect("agent event time")
        - Duration::seconds(1);
    let end = stored
        .iter()
        .filter(|event| event.source == source)
        .map(|event| event.observed_at)
        .max()
        .expect("agent event time")
        + Duration::seconds(1);
    let counts = store
        .window_counts(start, end, BucketWidth::Minute, Some(&source))
        .await
        .expect("window query");
    let total: i64 = counts.iter().map(|count| count.events).sum();
    assert_eq!(total, 2);
    assert!(counts.iter().all(|count| count.events > 0));
    let mut by_kind = BTreeMap::new();
    for count in counts {
        *by_kind.entry(count.kind).or_insert(0) += count.events;
    }
    assert_eq!(by_kind.get("agent.decide").copied(), Some(1));
    assert_eq!(by_kind.get("ledger.agent.pay_bill").copied(), Some(1));
}
