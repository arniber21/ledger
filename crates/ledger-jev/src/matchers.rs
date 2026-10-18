//! Exact-match and deterministic fuzzy baselines.
//!
//! Neither matcher calls Jev. The fuzzy matcher compares normalized vendor
//! names plus amount and currency, and ignores bill id and date. That is the
//! duplicate detector that collapses weekly rent. Exact match requires the
//! written vendor name, amount, currency, and every shared identity field.

use crate::obligation::{filled, trimmed, ObligationRef};
use crate::BindingVerdict;

/// Vendor-name similarity at or above this is a fuzzy match.
///
/// Fixed before the baseline System One calls. This cutoff belongs to the
/// deterministic matcher. Jev still uses [`crate::SAME_PROBABILITY_THRESHOLD`]
/// and [`crate::DIFFERENT_PROBABILITY_THRESHOLD`].
pub const FUZZY_NAME_THRESHOLD: f64 = 0.90;

/// `Same` when the written fields and identity fields agree.
///
/// A one-sided id, or any disagreeing id, is `Different`.
#[must_use = "an exact-match baseline is part of the comparison"]
pub fn exact_match(left: &ObligationRef, right: &ObligationRef) -> BindingVerdict {
    if trimmed(&left.vendor_name) == trimmed(&right.vendor_name)
        && trimmed(&left.amount) == trimmed(&right.amount)
        && trimmed(&left.currency) == trimmed(&right.currency)
        && identity_agrees(left, right)
    {
        BindingVerdict::Same
    } else {
        BindingVerdict::Different
    }
}

/// `Same` when amount, currency, and normalized vendor name agree.
///
/// Bill id and transaction date are ignored, so two weekly rent bills match.
#[must_use = "a fuzzy baseline is part of the comparison"]
pub fn fuzzy_match(left: &ObligationRef, right: &ObligationRef) -> BindingVerdict {
    if amounts_equal(&left.amount, &right.amount)
        && currencies_equal(&left.currency, &right.currency)
        && name_similarity(&left.vendor_name, &right.vendor_name) >= FUZZY_NAME_THRESHOLD
    {
        BindingVerdict::Same
    } else {
        BindingVerdict::Different
    }
}

fn identity_agrees(left: &ObligationRef, right: &ObligationRef) -> bool {
    let bill = field_agrees(&left.bill_id, &right.bill_id);
    let invoice = field_agrees(&left.invoice_number, &right.invoice_number);
    let vendor = field_agrees(&left.vendor_id, &right.vendor_id);
    let shared = both_present(&left.bill_id, &right.bill_id)
        || both_present(&left.invoice_number, &right.invoice_number)
        || both_present(&left.vendor_id, &right.vendor_id);
    bill && invoice && vendor && shared
}

fn field_agrees(left: &Option<String>, right: &Option<String>) -> bool {
    match (filled(left), filled(right)) {
        (Some(left), Some(right)) => left == right,
        (None, None) => true,
        _ => false,
    }
}

fn both_present(left: &Option<String>, right: &Option<String>) -> bool {
    filled(left).is_some() && filled(right).is_some()
}

fn amounts_equal(left: &str, right: &str) -> bool {
    match (
        trimmed(left).parse::<ledger_core::dataset::Money>(),
        trimmed(right).parse::<ledger_core::dataset::Money>(),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn currencies_equal(left: &str, right: &str) -> bool {
    match (nonempty(left), nonempty(right)) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(right),
        _ => false,
    }
}

fn nonempty(value: &str) -> Option<&str> {
    let text = trimmed(value);
    (!text.is_empty()).then_some(text)
}

fn name_similarity(left: &str, right: &str) -> f64 {
    let left = normalize_name(left);
    let right = normalize_name(right);
    if left == right {
        return 1.0;
    }
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let max_len = left.len().max(right.len());
    if max_len == 0 {
        return 1.0;
    }
    let distance = levenshtein(&left, &right);
    let max_len = u32::try_from(max_len).expect("vendor name length fits u32");
    let distance = u32::try_from(distance).expect("edit distance fits u32");
    1.0 - f64::from(distance) / f64::from(max_len)
}

fn normalize_name(name: &str) -> String {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            current.push(character.to_ascii_lowercase());
        } else if !current.is_empty() {
            push_token(&mut tokens, &mut current);
        }
    }
    if !current.is_empty() {
        push_token(&mut tokens, &mut current);
    }
    tokens.join(" ")
}

fn push_token(tokens: &mut Vec<String>, current: &mut String) {
    let token = std::mem::take(current);
    if !is_legal_suffix(&token) {
        tokens.push(token);
    }
}

fn is_legal_suffix(token: &str) -> bool {
    matches!(
        token,
        "inc" | "llc" | "ltd" | "co" | "company" | "corp" | "incorporated"
    )
}

fn levenshtein(left: &[char], right: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];
    for (row, left_char) in left.iter().enumerate() {
        current[0] = row + 1;
        for (column, right_char) in right.iter().enumerate() {
            let cost = usize::from(left_char != right_char);
            current[column + 1] = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(previous[column] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abbreviation_is_below_the_fuzzy_threshold() {
        let left = ObligationRef::new("Acme Industrial Supply", "1250.00", "USD")
            .with_invoice_number("INV-1044");
        let right =
            ObligationRef::new("Acme Ind. Supply", "1250.00", "USD").with_invoice_number("1044");
        assert_eq!(exact_match(&left, &right), BindingVerdict::Different);
        assert_eq!(fuzzy_match(&left, &right), BindingVerdict::Different);
    }

    #[test]
    fn identical_written_fields_match_exactly() {
        let left = ObligationRef::new("Ironclad Larkspur Outfitters", "4098.00", "USD")
            .with_vendor_id("vend-supplier")
            .with_bill_id("bill-00005")
            .with_invoice_number("BILL-5001");
        let right = left.clone();
        assert_eq!(exact_match(&left, &right), BindingVerdict::Same);
        assert_eq!(fuzzy_match(&left, &right), BindingVerdict::Same);
    }

    #[test]
    fn weekly_rent_is_exact_different_and_fuzzy_same() {
        let left = ObligationRef::new("Granite Bay Commercial Properties", "500.00", "USD")
            .with_vendor_id("vend-landlord")
            .with_bill_id("bill-operating-00003");
        let right = left.clone().with_bill_id("bill-operating-00018");
        assert_eq!(exact_match(&left, &right), BindingVerdict::Different);
        assert_eq!(fuzzy_match(&left, &right), BindingVerdict::Same);
    }
}
