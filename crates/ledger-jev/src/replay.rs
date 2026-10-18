//! Replay of recorded System One bodies.
//!
//! `LEDGER_JEV=replay` serves a saved response from `fixtures/jev/`. The key is
//! the question id plus the SHA-256 of the canonical request JSON. A missing
//! fixture is an error. Replay does not call out.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Why a replay lookup could not return a saved body.
#[derive(Debug, Error)]
pub enum ReplayError {
    /// A fixture file could not be read.
    #[error("replay fixtures could not be read: {0}")]
    Io(#[from] std::io::Error),

    /// The request was not a JSON object.
    #[error("replay request is not json")]
    Request,

    /// The request named no questions.
    #[error("replay request has no questions")]
    NoQuestion,

    /// No saved body matches this question and request.
    ///
    /// Replay stops here. It does not open a socket.
    #[error("replay fixture missing for question {question} input {hash}; refusing to call out")]
    Missing {
        /// Question id from the request.
        question: String,
        /// SHA-256 of the canonical request JSON.
        hash: String,
    },
}

/// Saved System One responses, keyed by question id and request hash.
#[derive(Clone, Debug)]
pub struct ReplayCatalog {
    bodies: BTreeMap<(String, String), String>,
}

impl ReplayCatalog {
    /// Loads `fixtures/jev/` next to this crate.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayError::Io`] when a fixture file cannot be read.
    pub fn load_default() -> Result<Self, ReplayError> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jev");
        Self::load(&dir)
    }

    /// Loads request/response pairs under `dir`.
    ///
    /// A file ending in `-request.json` pairs with `-response.json` or with
    /// the same stem and a `.json` suffix. The response body is stored once
    /// per question id in that request.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayError::Io`] when a file cannot be read.
    pub fn load(dir: &Path) -> Result<Self, ReplayError> {
        let mut files = Vec::new();
        walk(dir, &mut files);
        let mut requests: BTreeMap<(PathBuf, String), String> = BTreeMap::new();
        let mut responses: BTreeMap<(PathBuf, String), String> = BTreeMap::new();
        for path in files {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let Some(parent) = path.parent().map(Path::to_path_buf) else {
                continue;
            };
            let Some((stem, is_request)) = stem_kind(name) else {
                continue;
            };
            let text = fs::read_to_string(&path)?;
            let key = (parent, stem.to_owned());
            if is_request {
                requests.insert(key, text);
            } else {
                responses.entry(key).or_insert(text);
            }
        }
        let mut bodies = BTreeMap::new();
        for (key, request) in requests {
            let Some(response) = responses.get(&key) else {
                continue;
            };
            let Ok(hash) = request_hash(&request) else {
                continue;
            };
            for question in question_ids(&request) {
                bodies.insert((question, hash.clone()), response.clone());
            }
        }
        Ok(Self { bodies })
    }

    /// Returns the saved body for `request`.
    ///
    /// # Errors
    ///
    /// Returns [`ReplayError::Missing`] when any question in `request` has no
    /// saved body. The error names the question and the input hash. No call
    /// is made.
    pub fn require<'a>(&'a self, request: &str) -> Result<&'a str, ReplayError> {
        let hash = request_hash(request)?;
        let questions = question_ids(request);
        if questions.is_empty() {
            return Err(ReplayError::NoQuestion);
        }
        let mut found: Option<&str> = None;
        for question in questions {
            let Some(body) = self.bodies.get(&(question.clone(), hash.clone())) else {
                return Err(ReplayError::Missing { question, hash });
            };
            if let Some(previous) = found {
                if previous != body.as_str() {
                    return Err(ReplayError::Missing { question, hash });
                }
            }
            found = Some(body.as_str());
        }
        found.ok_or(ReplayError::NoQuestion)
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "json") {
            out.push(path);
        }
    }
}

fn stem_kind(name: &str) -> Option<(&str, bool)> {
    if let Some(stem) = name.strip_suffix("-request.json") {
        return Some((stem, true));
    }
    if let Some(stem) = name.strip_suffix("-response.json") {
        return Some((stem, false));
    }
    let stem = name.strip_suffix(".json")?;
    if stem.ends_with("-request") || stem.ends_with("-response") {
        return None;
    }
    Some((stem, false))
}

fn question_ids(request: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(request) else {
        return Vec::new();
    };
    let Some(questions) = value.get("questions").and_then(Value::as_object) else {
        return Vec::new();
    };
    questions.keys().cloned().collect()
}

fn request_hash(request: &str) -> Result<String, ReplayError> {
    let value: Value = serde_json::from_str(request).map_err(|_| ReplayError::Request)?;
    let text = serde_json::to_string(&sort_json(value)).map_err(|_| ReplayError::Request)?;
    let digest = Sha256::digest(text.as_bytes());
    Ok(hex_encode(&digest))
}

fn sort_json(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = BTreeMap::new();
            for (key, child) in map {
                sorted.insert(key, sort_json(child));
            }
            let mut object = Map::new();
            for (key, child) in sorted {
                object.insert(key, child);
            }
            Value::Object(object)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_json).collect()),
        other => other,
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        binding_request, map_memo_body, memo_request, ObligationRef, PredicateVerdict, SystemOne,
        SystemOneClient, INSTRUCTION_MEMO,
    };

    #[test]
    fn saved_memo_request_replays_without_a_socket() {
        let catalog = ReplayCatalog::load_default().expect("fixtures");
        let body = catalog
            .require(&memo_request(INSTRUCTION_MEMO))
            .expect("saved memo");
        assert_eq!(map_memo_body(body), PredicateVerdict::Instructs);
        assert!(!body.contains("TYPESAFE_API_KEY"));
        assert!(!body.contains("Bearer"));
    }

    #[test]
    fn a_missing_fixture_fails_and_does_not_call_out() {
        let catalog = ReplayCatalog::load_default().expect("fixtures");
        let request = serde_json::json!({
            "model": "jev-latest",
            "state": { "memo": "not a saved scene" },
            "questions": { "memo_instructs_payment": { "type": "noul" } }
        })
        .to_string();
        let error = catalog.require(&request).expect_err("missing");
        let text = error.to_string();
        assert!(text.contains("refusing to call out"), "{text}");
        assert!(text.contains("memo_instructs_payment"), "{text}");
        assert!(!text.contains("TYPESAFE_API_KEY"));
        assert!(!text.to_ascii_lowercase().contains("http"));
    }

    /// Writes the landlord-alias fixture from one live System One call.
    ///
    /// `cargo test -p ledger-jev --lib record_landlord_alias -- --ignored --exact`
    /// Replay of `just scene vendor-alias` reads the file this leaves behind.
    #[test]
    #[ignore]
    fn record_landlord_alias() {
        let key = std::env::var("TYPESAFE_API_KEY").expect("TYPESAFE_API_KEY");
        let client = SystemOneClient::new(key.clone()).expect("client");
        let left = ObligationRef::new("Landlord LLC", "500.00", "USD");
        let right = ObligationRef::new("Landlord L.L.C.", "500.00", "USD");
        let request = binding_request(&left, &right);
        assert!(!request.contains(&key));
        let response = client.evaluate(&request).expect("system one");
        assert!(!response.contains(&key));
        assert!(!response.contains("Bearer"));
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/jev/bindings");
        fs::write(root.join("landlord-alias-request.json"), &request).expect("request");
        fs::write(
            root.join("landlord-alias-response.json"),
            strip_explanation(&response),
        )
        .expect("response");
    }

    fn strip_explanation(body: &str) -> String {
        let Ok(mut value) = serde_json::from_str::<Value>(body) else {
            return body.to_owned();
        };
        fn walk(value: &mut Value) {
            match value {
                Value::Object(map) => {
                    map.retain(|key, _| key != "explanation");
                    for child in map.values_mut() {
                        walk(child);
                    }
                }
                Value::Array(items) => {
                    for item in items {
                        walk(item);
                    }
                }
                _ => {}
            }
        }
        walk(&mut value);
        value.to_string()
    }
}
