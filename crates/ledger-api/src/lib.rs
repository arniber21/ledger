//! HTTP/SSE API. Verdicts are `Satisfy`, `Violate`, or `Inconclusive`.
//!
//! `GET /v1/verdicts/stream` sends one JSON object per SSE `data` line:
//!
//! ```json
//! {
//!   "kind": "verdict",
//!   "verdict": {
//!     "id": "...",
//!     "outcome": "Violate",
//!     "title": "needs review",
//!     "specId": "ledger.bill.no-second-payment",
//!     "reasonCode": "second-payment",
//!     "evidence": "payment ... → second-payment ...",
//!     "timeline": [{ "eventId": "...", "role": "second-payment", "kind": "ledger.exchange.payment" }]
//!   }
//! }
//! ```
//!
//! The timeline is matched evidence in observation order. A `Violate` title is
//! `needs review`.
//!
//! `GET /api/traces` and `GET /api/traces/stream` read ingested spans.
//! `POST /api/faults/{name}` toggles a fault on the running desk.
//! `GET` and `POST /api/inbox/triage` read and write inbox status.
//! `POST /api/quick-actions` routes a phrase. It does not create a verdict.
//! `GET /api/calibration` counts stored inbox reviews by Jev score band.

mod calibration;
mod decode;
mod engine;
mod fault;
mod http;
mod inbox;
mod quick;
mod rank;
mod stream;
mod traces;

pub use engine::{
    ApiError, Engine, StreamBindingLine, StreamComparison, StreamEntry, StreamMessage,
    StreamVerdict,
};
pub use http::serve;
pub use http::serve_with_ranker;
pub use http::Server;
