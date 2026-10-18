//! HTTP/SSE API serving live verdicts and traces to the web UI.

use std::process::ExitCode;

use std::sync::Arc;

use ledger_api::serve_with_ranker;
use ledger_jev::{SystemOne, SystemOneClient};
use ledger_store::{database_url, Store};

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("api: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), ledger_api::ApiError> {
    let store = Store::connect(&database_url()).await?;
    store.migrate().await?;
    let bind = std::env::var("API_BIND")
        .or_else(|_| std::env::var("LEDGER_API_ADDR"))
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    // The blocking Jev client owns its own runtime. Build it off this one.
    let ranker = tokio::task::spawn_blocking(ranker_from_env)
        .await
        .unwrap_or(None);
    let server = serve_with_ranker(store, &bind, ranker).await?;
    println!("verdicts at http://{}/v1/verdicts/stream", server.addr);
    println!("inbox at http://{}/api/inbox/triage", server.addr);
    println!("calibration at http://{}/api/calibration", server.addr);
    println!("traces at http://{}/api/traces", server.addr);
    println!("quick actions at http://{}/api/quick-actions", server.addr);
    if std::env::var("LEDGER_TEST").ok().as_deref() == Some("1") {
        println!("test api at http://{}/api/test", server.addr);
    }
    server.wait().await;
    Ok(())
}

fn ranker_from_env() -> Option<Arc<dyn SystemOne>> {
    let Ok(key) = std::env::var("TYPESAFE_API_KEY") else {
        return None;
    };
    if key.is_empty() {
        return None;
    }
    match SystemOneClient::new(key) {
        Ok(client) => Some(Arc::new(client)),
        Err(error) => {
            tracing::warn!(error = %error, "jev ranking client was not started");
            None
        }
    }
}
