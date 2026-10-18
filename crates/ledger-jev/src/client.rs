//! Blocking System One client.
//!
//! The API key stays in the `Authorization` header. It is not written into
//! errors, [`Debug`] output, or the request body.

use crate::{JevCallFailure, CALL_TIMEOUT};

/// Evaluation endpoint. `model` in the body selects Jev.
pub const SYSTEM_ONE_URL: &str = "https://api.typesafe.ai/v1/systemone";

/// Something that can run one System One request.
pub trait SystemOne: Send + Sync {
    /// Posts `request` and returns the raw response body.
    ///
    /// # Errors
    ///
    /// Returns [`JevCallFailure`] on timeout, transport failure, a non-success
    /// status, or a body that is not UTF-8. The error does not include the
    /// API key or the response body.
    fn evaluate(&self, request: &str) -> Result<String, JevCallFailure>;
}

/// HTTP client for [`SYSTEM_ONE_URL`].
pub struct SystemOneClient {
    api_key: String,
    url: String,
    http: reqwest::blocking::Client,
}

impl std::fmt::Debug for SystemOneClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SystemOneClient")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl SystemOneClient {
    /// Builds a client for `api_key`.
    ///
    /// # Errors
    ///
    /// Returns [`JevCallFailure::Transport`] when the HTTP client cannot be built.
    pub fn new(api_key: impl Into<String>) -> Result<Self, JevCallFailure> {
        Self::with_url(api_key, SYSTEM_ONE_URL)
    }

    /// Builds a client for `url`. Tests can point this at a local server.
    ///
    /// # Errors
    ///
    /// Returns [`JevCallFailure::Transport`] when the HTTP client cannot be built.
    pub fn with_url(
        api_key: impl Into<String>,
        url: impl Into<String>,
    ) -> Result<Self, JevCallFailure> {
        let http = reqwest::blocking::Client::builder()
            .timeout(CALL_TIMEOUT)
            .build()
            .map_err(|_| JevCallFailure::Transport)?;
        Ok(Self {
            api_key: api_key.into(),
            url: url.into(),
            http,
        })
    }
}

impl SystemOne for SystemOneClient {
    fn evaluate(&self, request: &str) -> Result<String, JevCallFailure> {
        let response = self
            .http
            .post(&self.url)
            .bearer_auth(&self.api_key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(request.to_owned())
            .send()
            .map_err(classify)?;
        let status = response.status();
        let bytes = response.bytes().map_err(classify)?;
        if !status.is_success() {
            return Err(JevCallFailure::Http {
                status: status.as_u16(),
            });
        }
        String::from_utf8(bytes.to_vec()).map_err(|_| JevCallFailure::Utf8)
    }
}

fn classify(error: reqwest::Error) -> JevCallFailure {
    if error.is_timeout() {
        JevCallFailure::Timeout
    } else if let Some(status) = error.status() {
        JevCallFailure::Http {
            status: status.as_u16(),
        }
    } else {
        JevCallFailure::Transport
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_omits_the_api_key() {
        let client = SystemOneClient::new("super-secret-key").expect("client builds");
        let rendered = format!("{client:?}");
        assert!(!rendered.contains("super-secret-key"));
        assert!(rendered.contains(SYSTEM_ONE_URL));
    }
}
