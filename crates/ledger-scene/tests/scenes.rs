//! Each scene, twice, from an empty migrated database, in replay mode.

use std::collections::BTreeMap;
use std::env;

use ledger_scene::{run_scene, SceneReport, SCENES};
use ledger_store::{database_url, Store};
use sqlx::postgres::PgPoolOptions;

#[tokio::test]
async fn every_scene_replays_twice_from_an_empty_database() {
    let store = scene_database().await;
    store.truncate_runtime().await.expect("truncate");
    let first = run_show(&store).await;
    store.truncate_runtime().await.expect("truncate again");
    let second = run_show(&store).await;
    assert_eq!(first, second);
    expect(&first);
}

async fn run_show(store: &Store) -> BTreeMap<String, SceneReport> {
    env::set_var("LEDGER_JEV", "replay");
    let mut reports = BTreeMap::new();
    for name in SCENES {
        let report = run_scene(store, name)
            .await
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(report.name, name);
        assert!(!report.summary.contains('\n'));
        assert!(!report.summary.to_ascii_lowercase().contains("fraud"));
        reports.insert(name.to_owned(), report);
    }
    reports
}

fn expect(reports: &BTreeMap<String, SceneReport>) {
    let clean = &reports["clean"];
    assert!(clean.verdicts.iter().all(|item| item.outcome == "Satisfy"));
    assert!(clean.monitors.iter().all(|item| item.live == 0));
    assert!(clean.summary.contains("monitors flat at 0"));
    assert!(clean.summary.contains("dashboard events"));

    let double = &reports["double-pay"];
    assert!(double.verdicts.iter().any(|item| {
        item.outcome == "Violate"
            && item.reason == "second-payment"
            && item.roles == ["payment", "settlement", "second-payment"]
    }));
    assert_eq!(double.signatures.len(), 2);
    assert!(double.summary.contains("needs review"));
    assert!(double.summary.contains("counterexample 3"));

    let memo = &reports["hidden-memo"];
    assert!(memo.verdicts.iter().any(|item| {
        item.outcome == "Violate"
            && item.reason == "tainted-memo-without-approval"
            && item.roles.iter().any(|role| role == "memo")
    }));
    assert!(memo.summary.contains("needs review"));

    let alias = &reports["vendor-alias"];
    assert!(alias.summary.contains("exact "));
    assert!(alias.summary.contains("fuzzy "));
    assert!(alias.summary.contains("jev Same"));
    assert!(alias.summary.contains("comparison card shows all three"));

    let rent = &reports["recurring"];
    assert!(rent.summary.contains("52 weekly 500.00 USD"));
    assert!(rent.verdicts.iter().all(|item| item.outcome != "Violate"));
    assert!(rent.monitors.iter().all(|item| item.live == 0));

    let soak = &reports["gc-soak"];
    assert!(soak.summary.contains("flat at 15 against would-be 615"));
    assert!(soak
        .monitors
        .last()
        .is_some_and(|item| item.live == 15 && item.would_be == 615));

    let spec = &reports["spec-live"];
    assert!(spec.summary.contains("armed absence.after-until"));
    assert!(spec.summary.contains("jev Same"));
}

async fn scene_database() -> Store {
    let admin = database_url();
    let scene = scene_url(&admin);
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&admin)
        .await
        .expect("connect to timescale (just db-up)");
    let exists: Option<i32> =
        sqlx::query_scalar("SELECT 1 FROM pg_database WHERE datname = 'ledger_scene'")
            .fetch_optional(&pool)
            .await
            .expect("list databases");
    if exists.is_none() {
        sqlx::query("CREATE DATABASE ledger_scene")
            .execute(&pool)
            .await
            .expect("create ledger_scene");
    }
    drop(pool);
    let store = Store::connect(&scene)
        .await
        .expect("connect to ledger_scene");
    store.migrate().await.expect("migrate empty database");
    store
}

fn scene_url(admin: &str) -> String {
    match admin.rfind('/') {
        Some(index) => format!("{}/ledger_scene", &admin[..index]),
        None => format!("{admin}/ledger_scene"),
    }
}
