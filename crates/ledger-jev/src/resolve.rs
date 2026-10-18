//! Bind two references to monitor keys.
//!
//! Guards run first. Jev runs only when the guards are undecided. `Same`
//! yields one key. `Different` keeps two keys. `Unknown` is inconclusive and
//! does not merge.

use serde::{Deserialize, Serialize};

use crate::client::SystemOne;
use crate::guard::{guard, GuardDecision};
use crate::matchers::{exact_match, fuzzy_match};
use crate::obligation::{filled, ObligationRef};
use crate::request::binding_request;
use crate::{map_binding_body, BindingVerdict, JevCallFailure};

/// Why a System One call produced no stored body.
///
/// The response body is dropped. A non-success body can echo request headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CallFault {
    /// The call exceeded [`crate::CALL_TIMEOUT`].
    Timeout,
    /// The call failed before an HTTP status arrived.
    Transport,
    /// TypeSafe returned a non-success status.
    Http {
        /// HTTP status code.
        status: u16,
    },
    /// The response body was not UTF-8.
    Utf8,
}

impl From<JevCallFailure> for CallFault {
    fn from(failure: JevCallFailure) -> Self {
        match failure {
            JevCallFailure::Timeout => Self::Timeout,
            JevCallFailure::Transport => Self::Transport,
            JevCallFailure::Http { status } => Self::Http { status },
            JevCallFailure::Utf8 => Self::Utf8,
        }
    }
}

/// What the binding does to monitor keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[must_use]
pub enum BindingEffect {
    /// Both references share `key`.
    Merged {
        /// Canonical monitor key. A bill id wins over an invoice number.
        key: String,
    },
    /// The references stay on two keys.
    Distinct {
        /// Key for the left reference.
        left: String,
        /// Key for the right reference.
        right: String,
    },
    /// Jev was `Unknown`. The keys stay apart. This is not a pass.
    Inconclusive {
        /// Key for the left reference.
        left: String,
        /// Key for the right reference.
        right: String,
    },
}

impl BindingEffect {
    /// Whether the two references now share one monitor key.
    #[must_use]
    pub fn merged(&self) -> bool {
        matches!(self, Self::Merged { .. })
    }

    /// Monitor keys in left-then-right order. A merge is one key.
    #[must_use]
    pub fn monitor_keys(&self) -> Vec<&str> {
        match self {
            Self::Merged { key } => vec![key.as_str()],
            Self::Distinct { left, right } | Self::Inconclusive { left, right } => {
                vec![left.as_str(), right.as_str()]
            }
        }
    }
}

/// One binding decision, including the baselines and any Jev exchange.
///
/// `exact` and `fuzzy` are baselines. They do not choose [`Self::effect`].
/// `jev_request` and `jev_response` are present only when Jev ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Resolution {
    /// Guard decision, or the mapped Jev verdict when the guards were undecided.
    pub verdict: BindingVerdict,
    /// Merged, distinct, or inconclusive keys.
    pub effect: BindingEffect,
    /// What the guards decided before Jev.
    pub guard: GuardDecision,
    /// Exact-match baseline.
    pub exact: BindingVerdict,
    /// Fuzzy baseline.
    pub fuzzy: BindingVerdict,
    /// Mapped Jev verdict when Jev ran.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev: Option<BindingVerdict>,
    /// Whether [`resolve`] called Jev.
    pub called_jev: bool,
    /// Request body when Jev ran. No API key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_request: Option<String>,
    /// Response body when Jev returned one. Explanation fields are not added.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_response: Option<String>,
    /// Why Jev produced no body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_fault: Option<CallFault>,
    /// Left reference.
    pub left: ObligationRef,
    /// Right reference.
    pub right: ObligationRef,
}

impl Resolution {
    /// JSON record for the verdict detail. No model prose is added.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("resolution is finite JSON")
    }
}

/// Resolves `left` and `right`.
///
/// `jev` is called only when [`guard`] returns [`GuardDecision::Undecided`].
/// A missing client, a [`JevCallFailure`], or a low score becomes
/// [`BindingVerdict::Unknown`] and [`BindingEffect::Inconclusive`].
#[must_use = "the resolution holds the monitor keys"]
pub fn resolve(
    left: &ObligationRef,
    right: &ObligationRef,
    jev: Option<&dyn SystemOne>,
) -> Resolution {
    let exact = exact_match(left, right);
    let fuzzy = fuzzy_match(left, right);
    let decision = guard(left, right);
    match decision {
        GuardDecision::Same => Resolution {
            verdict: BindingVerdict::Same,
            effect: BindingEffect::Merged {
                key: canonical_key(left, right),
            },
            guard: decision,
            exact,
            fuzzy,
            jev: None,
            called_jev: false,
            jev_request: None,
            jev_response: None,
            jev_fault: None,
            left: left.clone(),
            right: right.clone(),
        },
        GuardDecision::Different => decided(
            left,
            right,
            decision,
            exact,
            fuzzy,
            BindingVerdict::Different,
            distinct(left, right),
        ),
        GuardDecision::Undecided => ask(left, right, decision, exact, fuzzy, jev),
    }
}

fn decided(
    left: &ObligationRef,
    right: &ObligationRef,
    decision: GuardDecision,
    exact: BindingVerdict,
    fuzzy: BindingVerdict,
    verdict: BindingVerdict,
    effect: BindingEffect,
) -> Resolution {
    Resolution {
        verdict,
        effect,
        guard: decision,
        exact,
        fuzzy,
        jev: None,
        called_jev: false,
        jev_request: None,
        jev_response: None,
        jev_fault: None,
        left: left.clone(),
        right: right.clone(),
    }
}

fn ask(
    left: &ObligationRef,
    right: &ObligationRef,
    decision: GuardDecision,
    exact: BindingVerdict,
    fuzzy: BindingVerdict,
    jev: Option<&dyn SystemOne>,
) -> Resolution {
    let Some(client) = jev else {
        return inconclusive(left, right, decision, exact, fuzzy, None, None);
    };
    let request = binding_request(left, right);
    match client.evaluate(&request) {
        Ok(body) => {
            let body = without_explanation(&body);
            let verdict = map_binding_body(&body);
            let effect = effect_for(verdict, left, right);
            Resolution {
                verdict,
                effect,
                guard: decision,
                exact,
                fuzzy,
                jev: Some(verdict),
                called_jev: true,
                jev_request: Some(request),
                jev_response: Some(body),
                jev_fault: None,
                left: left.clone(),
                right: right.clone(),
            }
        }
        Err(failure) => inconclusive(
            left,
            right,
            decision,
            exact,
            fuzzy,
            Some(request),
            Some(failure),
        ),
    }
}

fn inconclusive(
    left: &ObligationRef,
    right: &ObligationRef,
    decision: GuardDecision,
    exact: BindingVerdict,
    fuzzy: BindingVerdict,
    request: Option<String>,
    failure: Option<JevCallFailure>,
) -> Resolution {
    Resolution {
        verdict: BindingVerdict::Unknown,
        effect: BindingEffect::Inconclusive {
            left: own_key(left),
            right: disambiguated(right, left),
        },
        guard: decision,
        exact,
        fuzzy,
        jev: failure.as_ref().map(|_| BindingVerdict::Unknown),
        called_jev: request.is_some(),
        jev_request: request,
        jev_response: None,
        jev_fault: failure.map(CallFault::from),
        left: left.clone(),
        right: right.clone(),
    }
}

fn effect_for(
    verdict: BindingVerdict,
    left: &ObligationRef,
    right: &ObligationRef,
) -> BindingEffect {
    match verdict {
        BindingVerdict::Same => BindingEffect::Merged {
            key: canonical_key(left, right),
        },
        BindingVerdict::Different => distinct(left, right),
        BindingVerdict::Unknown => BindingEffect::Inconclusive {
            left: own_key(left),
            right: disambiguated(right, left),
        },
    }
}

fn without_explanation(body: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(body) else {
        return body.to_owned();
    };
    strip_explanation(&mut value);
    serde_json::to_string(&value).unwrap_or_else(|_| body.to_owned())
}

fn strip_explanation(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.retain(|key, _| key != "explanation");
            for child in map.values_mut() {
                strip_explanation(child);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                strip_explanation(item);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}

fn distinct(left: &ObligationRef, right: &ObligationRef) -> BindingEffect {
    BindingEffect::Distinct {
        left: own_key(left),
        right: disambiguated(right, left),
    }
}

fn canonical_key(left: &ObligationRef, right: &ObligationRef) -> String {
    filled(&left.bill_id)
        .or_else(|| filled(&right.bill_id))
        .map(str::to_owned)
        .unwrap_or_else(|| own_key(left))
}

fn own_key(reference: &ObligationRef) -> String {
    if let Some(id) = filled(&reference.bill_id) {
        return id.to_owned();
    }
    if let Some(id) = filled(&reference.invoice_number) {
        return format!("invoice:{id}");
    }
    if let Some(id) = filled(&reference.vendor_id) {
        return format!("vendor:{id}");
    }
    format!("name:{}", reference.vendor_name.trim())
}

/// Right-hand key. A collision gets a `#2` suffix so a non-merge cannot share a slice.
fn disambiguated(reference: &ObligationRef, other: &ObligationRef) -> String {
    let key = own_key(reference);
    if key == own_key(other) {
        format!("{key}#2")
    } else {
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::JevCallFailure;

    struct Reply(String);

    impl SystemOne for Reply {
        fn evaluate(&self, _request: &str) -> Result<String, JevCallFailure> {
            Ok(self.0.clone())
        }
    }

    struct Boom;

    impl SystemOne for Boom {
        fn evaluate(&self, _request: &str) -> Result<String, JevCallFailure> {
            panic!("jev must not be called once the guards decide");
        }
    }

    struct Fail(JevCallFailure);

    impl SystemOne for Fail {
        fn evaluate(&self, _request: &str) -> Result<String, JevCallFailure> {
            Err(self.0)
        }
    }

    fn choice(label: &str, same: f64, different: f64) -> String {
        serde_json::json!({
            "model": "jev-test",
            "answers": {
                "same_obligation": {
                    "type": "choice",
                    "choice": label,
                    "probabilities": { "same": same, "different": different }
                }
            }
        })
        .to_string()
    }

    fn abbreviated() -> (ObligationRef, ObligationRef) {
        let left = ObligationRef::new("Acme Industrial Supply", "1250.00", "USD")
            .with_invoice_number("INV-1044")
            .with_memo("Net-30 parts order for the March plant shutdown");
        let right = ObligationRef::new("Acme Ind. Supply", "1250.00", "USD")
            .with_invoice_number("1044")
            .with_memo("Parts for March shutdown, terms net 30");
        (left, right)
    }

    #[test]
    fn guards_skip_jev_on_a_shared_bill() {
        let left = ObligationRef::new("Granite Bay Commercial Properties", "500.00", "USD")
            .with_bill_id("bill-operating-00003");
        let right = left.clone().with_memo("other wording");
        let resolution = resolve(&left, &right, Some(&Boom));
        assert_eq!(resolution.verdict, BindingVerdict::Same);
        assert!(resolution.effect.merged());
        assert_eq!(resolution.effect.monitor_keys(), ["bill-operating-00003"]);
        assert!(!resolution.called_jev);
        assert!(resolution.jev_request.is_none());
    }

    #[test]
    fn jev_same_merges_onto_the_books_invoice() {
        let (left, right) = abbreviated();
        let resolution = resolve(&left, &right, Some(&Reply(choice("same", 0.91, 0.09))));
        assert_eq!(resolution.verdict, BindingVerdict::Same);
        assert_eq!(resolution.effect.monitor_keys(), ["invoice:INV-1044"]);
        assert!(resolution.called_jev);
        assert!(resolution.jev_request.is_some());
        assert!(resolution.jev_response.is_some());
        assert!(!resolution.to_json().contains("fraud"));
        assert!(!resolution
            .to_json()
            .to_ascii_lowercase()
            .contains("explain"));
    }

    #[test]
    fn timeout_is_inconclusive_and_does_not_merge() {
        let (left, right) = abbreviated();
        let resolution = resolve(&left, &right, Some(&Fail(JevCallFailure::Timeout)));
        assert_eq!(resolution.verdict, BindingVerdict::Unknown);
        assert!(!resolution.effect.merged());
        assert!(matches!(
            resolution.effect,
            BindingEffect::Inconclusive { .. }
        ));
        assert_eq!(resolution.effect.monitor_keys().len(), 2);
        assert_eq!(resolution.jev_fault, Some(CallFault::Timeout));
        assert!(resolution.jev_response.is_none());
    }

    #[test]
    fn missing_client_is_inconclusive() {
        let (left, right) = abbreviated();
        let resolution = resolve(&left, &right, None);
        assert_eq!(resolution.verdict, BindingVerdict::Unknown);
        assert!(!resolution.called_jev);
        assert!(!resolution.effect.merged());
    }

    #[test]
    fn low_score_does_not_merge() {
        let (left, right) = abbreviated();
        let resolution = resolve(&left, &right, Some(&Reply(choice("same", 0.70, 0.30))));
        assert_eq!(resolution.verdict, BindingVerdict::Unknown);
        assert!(matches!(
            resolution.effect,
            BindingEffect::Inconclusive { .. }
        ));
    }
}
