//! Ambiguous monitor closes: a credit or cancellation memo, and an alias merge.
//!
//! Deterministic closes stay in `ledger-core`. This module runs only when
//! those did not decide. The memo Noul uses [`crate::map_noul_question`] and
//! the fixed 0.80 thresholds. Alias merge uses [`crate::resolve`]. Unknown,
//! timeout, and a score under the threshold keep the monitor. This module
//! does not ask for or keep explanation text, and it does not call the network.

use std::collections::BTreeMap;

use ledger_core::{CloseJudgment, Decision, MonitorDesk};
use serde_json::{Map, Value};

use crate::memo::map_noul_question;
use crate::resolve::resolve;
use crate::{BindingEffect, BindingVerdict, JevCallFailure, PredicateVerdict, SystemOne};

/// Question id for "does this memo close the obligation?".
///
/// The id is for this crate. The model sees [`close_memo_request`]'s
/// instructions and criteria, not this string.
pub const MEMO_CLOSES_QUESTION: &str = "memo_closes_obligation";

/// Whether a credit or cancellation memo closes the obligation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum CloseVerdict {
    /// `P(yes)` is at or above the fixed 0.80 threshold.
    ///
    /// The monitor may retire. The finding still needs review.
    Closes,
    /// `P(no)` is at or above the fixed 0.80 threshold.
    ///
    /// The monitor stays. This is not a pass.
    StaysOpen,
    /// Timeout, error, a score under the threshold, or a contradiction.
    ///
    /// The monitor stays. This is not a pass.
    Unknown,
}

impl From<CloseVerdict> for CloseJudgment {
    fn from(verdict: CloseVerdict) -> Self {
        match verdict {
            CloseVerdict::Closes => Self::Affirm,
            CloseVerdict::StaysOpen => Self::Deny,
            CloseVerdict::Unknown => Self::Unknown,
        }
    }
}

impl From<BindingVerdict> for CloseJudgment {
    fn from(verdict: BindingVerdict) -> Self {
        match verdict {
            BindingVerdict::Same => Self::Affirm,
            BindingVerdict::Different => Self::Deny,
            BindingVerdict::Unknown => Self::Unknown,
        }
    }
}

/// Body of one close-memo Noul. The body has no API key.
#[must_use]
pub fn close_memo_request(memo: &str) -> String {
    let value = sort_objects(serde_json::json!({
        "state": {
            "source": "bill.memo",
            "memo": memo
        },
        "model": "jev-latest",
        "questions": {
            MEMO_CLOSES_QUESTION: {
                "type": "noul",
                "instructions": "Does `memo` close the obligation, as a credit or a cancellation?",
                "criteria": {
                    "true": "The memo is a credit, a cancellation, or a statement that the amount owed is finished.",
                    "false": "The memo does not close the obligation. A description of goods, a payment instruction, or an unclear note leaves it open."
                }
            }
        }
    }));
    serde_json::to_string(&value).expect("close memo request is finite JSON")
}

/// Map a raw System One Noul for [`MEMO_CLOSES_QUESTION`].
///
/// Invalid JSON, a missing or non-noul answer, a probability outside
/// `[0, 1]`, a score under the fixed 0.80 thresholds, and a contradiction all
/// return [`CloseVerdict::Unknown`]. Any explanation field is ignored.
#[must_use = "an unmapped Noul is dropped"]
pub fn map_close_memo_body(body: &str) -> CloseVerdict {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return CloseVerdict::Unknown;
    };
    match map_noul_question(&value, MEMO_CLOSES_QUESTION) {
        PredicateVerdict::Instructs => CloseVerdict::Closes,
        PredicateVerdict::DoesNot => CloseVerdict::StaysOpen,
        PredicateVerdict::Unknown => CloseVerdict::Unknown,
    }
}

/// Map a finished close-memo call.
///
/// Every [`JevCallFailure`] becomes [`CloseVerdict::Unknown`].
#[must_use = "an unmapped Noul is dropped"]
pub fn map_close_memo_exchange(exchange: Result<&str, JevCallFailure>) -> CloseVerdict {
    match exchange {
        Ok(body) => map_close_memo_body(body),
        Err(
            JevCallFailure::Timeout
            | JevCallFailure::Transport
            | JevCallFailure::Http { .. }
            | JevCallFailure::Utf8,
        ) => CloseVerdict::Unknown,
    }
}

/// Retire `key` only when the close memo maps to [`CloseVerdict::Closes`].
///
/// A blank memo is missing evidence and stays [`CloseJudgment::Unknown`].
///
/// # Errors
///
/// Returns an error when the desk rejects the retirement signal.
pub fn consider_close_memo(
    desk: &mut MonitorDesk,
    key: &str,
    memo: &str,
    exchange: Result<&str, JevCallFailure>,
    observed_at_ns: i128,
) -> Result<Decision, ledger_core::AdapterError> {
    let judgment = if memo.trim().is_empty() {
        CloseJudgment::Unknown
    } else {
        CloseJudgment::from(map_close_memo_exchange(exchange))
    };
    desk.judge_close(key, judgment, observed_at_ns)
}

/// Retire `alias` only when [`resolve`] says the two references are one obligation.
///
/// Guards run first and skip Jev when they decide. Unknown does not merge and
/// does not retire. The canonical key stays live when it already is.
///
/// # Errors
///
/// Returns an error when the desk rejects the retirement signal.
pub fn consider_alias(
    desk: &mut MonitorDesk,
    alias: &str,
    left: &crate::ObligationRef,
    right: &crate::ObligationRef,
    jev: Option<&dyn SystemOne>,
    observed_at_ns: i128,
) -> Result<Decision, ledger_core::AdapterError> {
    let resolution = resolve(left, right, jev);
    let canonical = match &resolution.effect {
        BindingEffect::Merged { key } => key.as_str(),
        BindingEffect::Distinct { .. } | BindingEffect::Inconclusive { .. } => alias,
    };
    desk.apply_alias(
        alias,
        canonical,
        CloseJudgment::from(resolution.verdict),
        observed_at_ns,
    )
}

fn sort_objects(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted = BTreeMap::new();
            for (key, child) in map {
                sorted.insert(key, sort_objects(child));
            }
            let mut object = Map::new();
            for (key, child) in sorted {
                object.insert(key, child);
            }
            Value::Object(object)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sort_objects).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obligation::ObligationRef;
    use crate::JevCallFailure;
    use ledger_core::{
        AgentId, BillId, LedgerEvent, LedgerKind, MonitorDesk, OpenBill, Times, VerdictOutcome,
    };
    use std::str::FromStr;

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

    fn noul(p_yes: f64) -> String {
        serde_json::json!({
            "model": "jev-test",
            "answers": {
                MEMO_CLOSES_QUESTION: {
                    "type": "noul",
                    "noul": p_yes
                }
            }
        })
        .to_string()
    }

    fn agent() -> AgentId {
        AgentId::new("payer").expect("agent")
    }

    fn open(desk: &mut MonitorDesk, key: &str) {
        desk.open_bill(
            OpenBill::new(key, agent(), 1)
                .with_horizon(10_000)
                .with_owed(
                    ledger_core::dataset::Money::from_str("500.00").expect("money"),
                    "USD",
                ),
        )
        .expect("open");
    }

    fn push_double_pay(desk: &mut MonitorDesk, bill: &str) {
        let bill_id = BillId::new(bill).expect("bill");
        let events = [
            LedgerEvent::new(
                "pay",
                LedgerKind::AgentPayBill,
                bill_id.clone(),
                agent(),
                Times::at(10),
                std::iter::empty::<&str>(),
            )
            .expect("pay"),
            LedgerEvent::new(
                "settled",
                LedgerKind::ObligationSettled,
                bill_id.clone(),
                agent(),
                Times::at(20),
                ["pay"],
            )
            .expect("settled"),
            LedgerEvent::new(
                "pay-again",
                LedgerKind::AgentPayBill,
                bill_id.clone(),
                agent(),
                Times::at(30),
                ["settled"],
            )
            .expect("again"),
            LedgerEvent::new(
                "extra",
                LedgerKind::ExtraPayment,
                bill_id.clone(),
                agent(),
                Times::at(40),
                ["pay-again"],
            )
            .expect("extra"),
            LedgerEvent::new(
                "credit",
                LedgerKind::Credit,
                bill_id,
                agent(),
                Times::at(50),
                ["extra"],
            )
            .expect("credit"),
        ];
        for event in events {
            desk.push(event).expect("push");
        }
    }

    #[test]
    fn closing_memo_uses_the_fixed_threshold() {
        assert_eq!(map_close_memo_body(&noul(0.80)), CloseVerdict::Closes);
        assert_eq!(map_close_memo_body(&noul(0.79)), CloseVerdict::Unknown);
        assert_eq!(map_close_memo_body(&noul(0.20)), CloseVerdict::StaysOpen);
        assert_eq!(
            map_close_memo_exchange(Err(JevCallFailure::Timeout)),
            CloseVerdict::Unknown
        );
        let mut answer = serde_json::json!({
            "type": "noul",
            "noul": 0.91
        });
        answer["explanation"] = Value::String("model text that must not be used".to_string());
        let raw = serde_json::json!({
            "answers": { MEMO_CLOSES_QUESTION: answer }
        })
        .to_string();
        assert_eq!(map_close_memo_body(&raw), CloseVerdict::Closes);
        assert!(!close_memo_request("Credit applied")
            .to_lowercase()
            .contains("explain"));
        assert!(!close_memo_request("Credit applied")
            .to_lowercase()
            .contains("fraud"));
    }

    #[test]
    fn unknown_memo_keeps_the_monitor_and_a_later_double_pay_is_caught() {
        let mut desk = MonitorDesk::new().expect("desk");
        open(&mut desk, "bill-1");
        let timeout = consider_close_memo(
            &mut desk,
            "bill-1",
            "Credit memo, possibly applied to another invoice",
            Err(JevCallFailure::Timeout),
            15,
        )
        .expect("timeout");
        assert!(matches!(timeout, Decision::Kept { .. }));
        assert!(desk.is_live("bill-1"));

        let low = consider_close_memo(
            &mut desk,
            "bill-1",
            "Credit memo, possibly applied to another invoice",
            Ok(&noul(0.79)),
            16,
        )
        .expect("low score");
        assert!(matches!(low, Decision::Kept { .. }));
        push_double_pay(&mut desk, "bill-1");
        assert_eq!(
            desk.absence_outcome("bill-1").expect("absence"),
            VerdictOutcome::Violate
        );
        assert_eq!(desk.live_count(), 1);
    }

    #[test]
    fn a_blank_memo_and_a_closing_score_follow_the_rule() {
        let mut desk = MonitorDesk::new().expect("desk");
        open(&mut desk, "bill-1");
        assert!(matches!(
            consider_close_memo(&mut desk, "bill-1", "   ", Err(JevCallFailure::Timeout), 15)
                .expect("blank"),
            Decision::Kept { .. }
        ));
        assert!(desk.is_live("bill-1"));
        let decision = consider_close_memo(
            &mut desk,
            "bill-1",
            "Credit applied. This invoice is cancelled.",
            Ok(&noul(0.80)),
            16,
        )
        .expect("close");
        assert!(matches!(decision, Decision::Retired { .. }));
        assert!(!desk.is_live("bill-1"));
        assert_eq!(
            desk.absence_outcome("bill-1").expect("retired"),
            VerdictOutcome::Inconclusive
        );
        push_double_pay(&mut desk, "bill-1");
        assert_eq!(desk.live_count(), 0);
        assert_eq!(
            desk.absence_outcome("bill-1").expect("still inconclusive"),
            VerdictOutcome::Inconclusive
        );
    }

    #[test]
    fn alias_timeout_keeps_the_monitor_and_a_later_double_pay_is_caught() {
        let mut desk = MonitorDesk::new().expect("desk");
        open(&mut desk, "invoice:INV-1044");
        let left = ObligationRef::new("Acme Industrial Supply", "1250.00", "USD")
            .with_invoice_number("INV-1044");
        let right =
            ObligationRef::new("Acme Ind. Supply", "1250.00", "USD").with_invoice_number("1044");
        let decision = consider_alias(
            &mut desk,
            "invoice:INV-1044",
            &left,
            &right,
            Some(&Fail(JevCallFailure::Timeout)),
            15,
        )
        .expect("alias");
        assert!(matches!(decision, Decision::Kept { .. }));
        assert_eq!(desk.live_count(), 1);
        push_double_pay(&mut desk, "invoice:INV-1044");
        assert_eq!(
            desk.absence_outcome("invoice:INV-1044").expect("absence"),
            VerdictOutcome::Violate
        );
    }

    #[test]
    fn guards_merge_without_jev_and_a_low_score_does_not() {
        let mut desk = MonitorDesk::new().expect("desk");
        open(&mut desk, "alias-bill");
        open(&mut desk, "bill-operating-00003");
        let left = ObligationRef::new("Granite Bay Commercial Properties", "500.00", "USD")
            .with_bill_id("bill-operating-00003");
        let right = left.clone();
        let decision = consider_alias(&mut desk, "alias-bill", &left, &right, Some(&Boom), 15)
            .expect("guards");
        assert!(matches!(decision, Decision::Retired { .. }));
        assert!(desk.is_live("bill-operating-00003"));
        assert!(!desk.is_live("alias-bill"));

        let (ambiguous_left, ambiguous_right) = {
            let left = ObligationRef::new("Acme Industrial Supply", "1250.00", "USD")
                .with_invoice_number("INV-1044");
            let right = ObligationRef::new("Acme Ind. Supply", "1250.00", "USD")
                .with_invoice_number("1044");
            (left, right)
        };
        open(&mut desk, "invoice:INV-9");
        let low = consider_alias(
            &mut desk,
            "invoice:INV-9",
            &ambiguous_left,
            &ambiguous_right,
            Some(&Reply(choice("same", 0.70, 0.30))),
            16,
        )
        .expect("low");
        assert!(matches!(low, Decision::Kept { .. }));
        assert!(desk.is_live("invoice:INV-9"));
    }

    struct Reply(String);

    impl SystemOne for Reply {
        fn evaluate(&self, _request: &str) -> Result<String, JevCallFailure> {
            Ok(self.0.clone())
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
}
