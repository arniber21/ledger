//! HTTP/SSE API serving live verdicts and traces to the web UI.

use std::process::ExitCode;

use ledger_api::serve;
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
    let server = serve(store, &bind).await?;
    println!("verdicts at http://{}/v1/verdicts/stream", server.addr);
    println!("inbox at http://{}/api/inbox/triage", server.addr);
    println!("calibration at http://{}/api/calibration", server.addr);
    println!("traces at http://{}/api/traces", server.addr);
    println!("quick actions at http://{}/api/quick-actions", server.addr);
    server.wait().await;
    Ok(())
}
