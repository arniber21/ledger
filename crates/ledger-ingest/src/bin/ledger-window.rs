//! Print per-minute event counts for the last hour.
//!
//! This is the query in the README. The bucket is Timescale `time_bucket`.

use std::process::ExitCode;

use chrono::{Duration, Utc};
use ledger_store::{database_url, BucketWidth, Store};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("window-query: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), ledger_store::StoreError> {
    let store = Store::connect(&database_url()).await?;
    store.migrate().await?;
    let end = Utc::now();
    let start = end - Duration::hours(1);
    let counts = store
        .window_counts(start, end, BucketWidth::Minute, None)
        .await?;
    println!("bucket\tsource\tkind\tevents");
    for count in counts {
        println!(
            "{}\t{}\t{}\t{}",
            count.bucket.to_rfc3339(),
            count.source,
            count.kind,
            count.events
        );
    }
    Ok(())
}
