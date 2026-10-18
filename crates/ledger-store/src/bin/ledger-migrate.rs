//! Apply the TimescaleDB migrations.

use std::process::ExitCode;

use ledger_store::{database_url, Store};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("migrate: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), ledger_store::StoreError> {
    let store = Store::connect(&database_url()).await?;
    store.migrate().await?;
    println!("migrations applied");
    Ok(())
}
