//! Runs one demo scene, or every scene in run-of-show order.
//!
//! `ledger-scene <name>` prints the one-line expectation.
//! `ledger-scene --all` runs them in order.
//! `LEDGER_JEV=replay` is the default. `LEDGER_JEV=live` calls System One.
//! The key is never printed.

use ledger_scene::{run_all, run_scene, SceneError, SCENES};
use ledger_store::{database_url, Store};

#[tokio::main]
async fn main() {
    let code = match run().await {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("scene: {error}");
            1
        }
    };
    std::process::exit(code);
}

async fn run() -> Result<(), SceneError> {
    let mut args = std::env::args().skip(1);
    let Some(name) = args.next() else {
        return Err(SceneError::Check(format!(
            "usage: ledger-scene <{}> | --all",
            SCENES.join("|")
        )));
    };
    if args.next().is_some() {
        return Err(SceneError::Check(
            "usage: ledger-scene <name> | --all".to_owned(),
        ));
    }
    let store = Store::connect(&database_url()).await?;
    store.migrate().await?;
    if name == "--all" {
        for report in run_all(&store).await? {
            println!("{}", report.summary);
        }
        return Ok(());
    }
    let report = run_scene(&store, &name).await?;
    println!("{}", report.summary);
    Ok(())
}
