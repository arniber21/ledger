//! Generates the pre-sanitization corpus input for the #78 semantic gate.
//!
//! This binary exists only for the regeneration recipe: it converts the
//! vendored SagaShop captures into generic `trace-corpus-v1` input records and
//! writes one `.jsonl` file per scenario under the output directory. It is
//! `#[ignore]`d from ordinary test runs — use `just corpus-regenerate` to run
//! it, which then feeds the output through `trace-grab` and vendors the
//! sanitized bundles.

#![allow(clippy::doc_markdown)] // Product names read as prose here.

mod corpus_support;

use std::path::PathBuf;

use corpus_support::{capture_to_corpus_records, records_to_jsonl};
use sentinel_ingest::capture::Capture;

fn fixture_path(scenario: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("testdata")
        .join("sagashop")
        .join(format!("ts-{scenario}.json"))
}

#[test]
#[ignore = "regeneration helper: run via `just corpus-regenerate`"]
fn write_corpus_inputs() {
    let output_dir = match std::env::var("SENTINEL_CORPUS_INPUT_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => return, // ordinary `cargo test -- --ignored` without the env: no-op
    };
    std::fs::create_dir_all(&output_dir).expect("output directory is creatable");
    for scenario in ["happy", "cancel", "hazards"] {
        let bytes = std::fs::read(fixture_path(scenario))
            .unwrap_or_else(|error| panic!("vendored {scenario} capture is readable: {error}"));
        let capture = Capture::from_json(&bytes)
            .unwrap_or_else(|error| panic!("vendored {scenario} capture decodes: {error}"));
        let records = capture_to_corpus_records(&capture)
            .unwrap_or_else(|error| panic!("{scenario} capture converts: {error}"));
        std::fs::write(
            output_dir.join(format!("{scenario}.jsonl")),
            records_to_jsonl(&records),
        )
        .unwrap_or_else(|error| panic!("{scenario} corpus input is writable: {error}"));
    }
}
