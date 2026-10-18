//! Scripted spans persisted through ingest.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use ledger_ingest::{persist, CapturedSpan};
use ledger_store::{Event, Store};
use solana_keypair::Keypair;
use solana_signer::Signer;
use uuid::Uuid;

use crate::clock::{self, SCENE_SEED};
use crate::SceneError;

const SCENE_NAMESPACE: Uuid = Uuid::from_u128(0x6c65_6467_6572_5f73_6365_6e65_0001);

/// Spans for one scene, in script order.
pub struct Script {
    scene: &'static str,
    spans: Vec<CapturedSpan>,
    next_span: u64,
}

impl Script {
    pub fn new(scene: &'static str) -> Self {
        Self {
            scene,
            spans: Vec::new(),
            next_span: 1,
        }
    }

    /// Stable id for `name` inside this scene.
    #[must_use]
    pub fn id(&self, name: &str) -> Uuid {
        Uuid::new_v5(
            &SCENE_NAMESPACE,
            format!("{}:{name}", self.scene).as_bytes(),
        )
    }

    pub fn add(
        &mut self,
        name: &'static str,
        id: Uuid,
        at: DateTime<Utc>,
        fields: &[(&str, &str)],
    ) {
        let mut map = BTreeMap::new();
        map.insert("scene".to_owned(), self.scene.to_owned());
        map.insert("event.id".to_owned(), id.to_string());
        for (key, value) in fields {
            if !value.trim().is_empty() {
                map.insert((*key).to_owned(), (*value).to_owned());
            }
        }
        let span_id = self.next_span;
        self.next_span = self.next_span.saturating_add(1);
        self.spans
            .push(CapturedSpan::new(name, span_id, None, at, map));
    }

    /// Writes the spans into the hypertable.
    ///
    /// # Errors
    ///
    /// Returns an error when an insert fails.
    pub async fn persist(&self, store: &Store) -> Result<Vec<Event>, SceneError> {
        Ok(persist(store, &self.spans).await?)
    }
}

/// Deterministic Solana signature of a scene label.
///
/// The key is the scene seed. The same label signs to the same signature on
/// every replay. This is not a validator confirmation and not a Sentry result.
#[must_use]
pub fn scene_signature(label: &str) -> String {
    let mut secret = [0_u8; 32];
    secret[..8].copy_from_slice(&SCENE_SEED.to_le_bytes());
    secret[8..20].copy_from_slice(b"ledger-scene");
    let keypair = Keypair::new_from_array(secret);
    keypair.sign_message(label.as_bytes()).to_string()
}

/// Scene clock plus a day offset, then `seconds` within that day.
#[must_use]
pub fn day(day: i64, seconds: i64) -> DateTime<Utc> {
    clock::at(day.saturating_mul(86_400).saturating_add(seconds))
}
