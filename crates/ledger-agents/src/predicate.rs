//! Live System One call for the memo predicate.
//!
//! Tests replay a saved body through [`crate::desk::Desk::replay_memo_predicate`]
//! and do not call this. The API key is `TYPESAFE_API_KEY`. It is not printed,
//! logged, or stored in the response.

use ledger_jev::{memo_request, JevCallFailure, CALL_TIMEOUT};

/// Posts one memo predicate and returns the raw response body.
///
/// A missing key, a timeout, a transport failure, or a non-success status is
/// a [`JevCallFailure`]. The failure does not include the key or the body.
pub(crate) async fn fetch_memo_predicate(memo: &str) -> Result<String, JevCallFailure> {
    let raw = std::env::var("TYPESAFE_API_KEY").unwrap_or_default();
    let key = raw.trim();
    if key.is_empty() {
        return Err(JevCallFailure::Transport);
    }
    let client = rig_reqwest::reqwest::Client::builder()
        .timeout(CALL_TIMEOUT)
        .build()
        .map_err(|_| JevCallFailure::Transport)?;
    let response = client
        .post("https://api.typesafe.ai/v1/systemone")
        .bearer_auth(key)
        .header("content-type", "application/json")
        .body(memo_request(memo))
        .send()
        .await
        .map_err(call_failure)?;
    let status = response.status();
    if !status.is_success() {
        return Err(JevCallFailure::Http {
            status: status.as_u16(),
        });
    }
    let text = response.text().await.map_err(call_failure)?;
    if text.contains(key) || text.contains(raw.as_str()) {
        return Err(JevCallFailure::Transport);
    }
    Ok(text)
}

fn call_failure(error: rig_reqwest::reqwest::Error) -> JevCallFailure {
    if error.is_timeout() {
        JevCallFailure::Timeout
    } else if error.is_decode() {
        JevCallFailure::Utf8
    } else {
        JevCallFailure::Transport
    }
}
