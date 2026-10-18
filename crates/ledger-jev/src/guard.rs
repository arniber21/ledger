//! Deterministic identity guards.
//!
//! Amount, bill id, vendor id, and currency decide before Jev. A decision
//! skips the model. Invoice punctuation and vendor-name wording do not decide:
//! those stay for Jev. Missing amount or currency does not count as a match.

use ledger_core::dataset::Money;
use serde::{Deserialize, Serialize};

use crate::obligation::{filled, trimmed, ObligationRef};

/// Whether the guards decided, and which way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[must_use]
pub enum GuardDecision {
    /// Amount, currency, and bill id agree. Vendor ids do not disagree.
    Same,
    /// Amount, currency, bill id, or vendor id disagrees.
    Different,
    /// The guards cannot decide. Jev may run.
    Undecided,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Relation {
    Equal,
    Disagree,
    Unknown,
}

/// Decides from amount, ids, and currency.
///
/// Same requires a shared bill id plus equal amount and currency. Distinct
/// bill ids are different obligations even when the vendor and amount match
/// (weekly rent). A shared vendor id alone does not decide Same.
#[must_use = "an undecided guard is why jev runs"]
pub fn guard(left: &ObligationRef, right: &ObligationRef) -> GuardDecision {
    let currency = currency_relation(&left.currency, &right.currency);
    let amount = amount_relation(&left.amount, &right.amount);
    let bill = id_relation(&left.bill_id, &right.bill_id);
    let vendor = id_relation(&left.vendor_id, &right.vendor_id);
    if currency == Relation::Disagree
        || amount == Relation::Disagree
        || bill == Relation::Disagree
        || vendor == Relation::Disagree
    {
        return GuardDecision::Different;
    }
    if amount == Relation::Equal && currency == Relation::Equal && bill == Relation::Equal {
        return GuardDecision::Same;
    }
    GuardDecision::Undecided
}

fn currency_relation(left: &str, right: &str) -> Relation {
    match (nonempty(left), nonempty(right)) {
        (Some(left), Some(right)) if left.eq_ignore_ascii_case(right) => Relation::Equal,
        (Some(_), Some(_)) => Relation::Disagree,
        _ => Relation::Unknown,
    }
}

fn amount_relation(left: &str, right: &str) -> Relation {
    match (parse_amount(left), parse_amount(right)) {
        (Some(left), Some(right)) if left == right => Relation::Equal,
        (Some(_), Some(_)) => Relation::Disagree,
        _ => Relation::Unknown,
    }
}

fn id_relation(left: &Option<String>, right: &Option<String>) -> Relation {
    match (filled(left), filled(right)) {
        (Some(left), Some(right)) if left == right => Relation::Equal,
        (Some(_), Some(_)) => Relation::Disagree,
        _ => Relation::Unknown,
    }
}

fn parse_amount(value: &str) -> Option<Money> {
    nonempty(value)?.parse::<Money>().ok()
}

fn nonempty(value: &str) -> Option<&str> {
    let text = trimmed(value);
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (ObligationRef, ObligationRef) {
        let left = ObligationRef::new("Granite Bay Commercial Properties", "500.00", "USD")
            .with_vendor_id("vend-landlord")
            .with_bill_id("bill-operating-00003")
            .with_invoice_number("BILL-5002");
        let right = left
            .clone()
            .with_bill_id("bill-operating-00018")
            .with_invoice_number("BILL-5004");
        (left, right)
    }

    #[test]
    fn distinct_bill_ids_are_different_even_when_vendor_and_amount_match() {
        let (left, right) = pair();
        assert_eq!(guard(&left, &right), GuardDecision::Different);
    }

    #[test]
    fn same_bill_id_amount_and_currency_skip_to_same() {
        let (left, _) = pair();
        let right = left.clone().with_memo("rewritten memo");
        assert_eq!(guard(&left, &right), GuardDecision::Same);
    }

    #[test]
    fn currency_disagreement_is_different() {
        let (left, _) = pair();
        let right = left.clone().with_bill_id("bill-operating-00003");
        let mut right = right;
        right.currency = "EUR".to_owned();
        assert_eq!(guard(&left, &right), GuardDecision::Different);
    }

    #[test]
    fn amount_disagreement_is_different() {
        let (left, _) = pair();
        let mut right = left.clone();
        right.amount = "4098.00".to_owned();
        assert_eq!(guard(&left, &right), GuardDecision::Different);
    }

    #[test]
    fn vendor_id_disagreement_is_different() {
        let (left, _) = pair();
        let right = left.clone().with_vendor_id("vend-supplier");
        assert_eq!(guard(&left, &right), GuardDecision::Different);
    }

    #[test]
    fn abbreviation_without_ids_is_undecided() {
        let left = ObligationRef::new("Acme Industrial Supply", "1250.00", "USD")
            .with_invoice_number("INV-1044");
        let right =
            ObligationRef::new("Acme Ind. Supply", "1250.00", "USD").with_invoice_number("1044");
        assert_eq!(guard(&left, &right), GuardDecision::Undecided);
    }

    #[test]
    fn missing_amount_does_not_count_as_same() {
        let left = ObligationRef::new("Acme Industrial Supply", "", "USD").with_bill_id("bill-1");
        let right = ObligationRef::new("Acme Industrial Supply", "", "USD").with_bill_id("bill-1");
        assert_eq!(guard(&left, &right), GuardDecision::Undecided);
    }
}
