//! HTTP server for the live trace view.

#[tokio::main]
async fn main() {
    if let Err(err) = ledger_api::serve().await {
        eprintln!("api: {err}");
        std::process::exit(1);
    }
}
