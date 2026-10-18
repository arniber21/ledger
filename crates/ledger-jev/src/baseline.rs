//! Three controlled comparisons of exact match, fuzzy match, and Jev.
//!
//! The report lists cases. It has no accuracy score. One case is the weekly
//! rent pair: the fuzzy matcher treats those bills as one obligation, and the
//! resolver does not.

use serde::{Deserialize, Serialize};

use crate::client::SystemOne;
use crate::guard::GuardDecision;
use crate::obligation::ObligationRef;
use crate::request::binding_request;
use crate::resolve::{resolve, BindingEffect};
use crate::{map_binding_body, BindingVerdict, JevCallFailure};

/// One controlled pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineCase {
    /// Stable case id.
    pub id: &'static str,
    /// What the pair is for.
    pub label: &'static str,
    /// Books side, or the left record.
    pub left: ObligationRef,
    /// Reference being bound.
    pub right: ObligationRef,
}

/// Exact, fuzzy, Jev, and the resolver for one case.
///
/// `jev_request` and `jev_response` are the stored System One exchange.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseComparison {
    /// Case id.
    pub id: String,
    /// Short label. No model prose.
    pub label: String,
    /// Exact-match baseline.
    pub exact: BindingVerdict,
    /// Fuzzy baseline.
    pub fuzzy: BindingVerdict,
    /// Mapped Jev verdict from the stored response.
    pub jev: BindingVerdict,
    /// Resolver verdict. Guards win when they decide.
    pub resolver: BindingVerdict,
    /// Guard decision.
    pub guard: GuardDecision,
    /// Monitor-key effect of the resolver.
    pub effect: BindingEffect,
    /// Jev's verdict differs from both baselines, and is not `Unknown`.
    pub jev_changed_finding: bool,
    /// Fuzzy returned `Same` for this pair.
    pub fuzzy_would_merge: bool,
    /// The resolver called Jev. False when a guard decided.
    pub resolver_called_jev: bool,
    /// Stored request body.
    pub jev_request: String,
    /// Stored response body.
    pub jev_response: String,
}

/// The three cases, in report order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineReport {
    cases: Vec<CaseComparison>,
}

impl BaselineReport {
    /// Cases in report order.
    #[must_use]
    pub fn cases(&self) -> &[CaseComparison] {
        &self.cases
    }

    /// JSON for the verdict detail. Cases only: there is no accuracy field.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("baseline report is finite JSON")
    }
}

/// The abbreviated-vendor case, the weekly-rent case, and two different bills.
///
/// Rent ids are company-001 bills `bill-operating-00003` and
/// `bill-operating-00018` (Granite Bay Commercial Properties, 500.00 USD, a
/// week apart). The different-bills case is `bill-00005` against the first of
/// those. The abbreviated case is the controlled pair from the first binding
/// call, not a dataset row.
#[must_use]
pub fn baseline_cases() -> [BaselineCase; 3] {
    [
        BaselineCase {
            id: "abbreviated-vendor",
            label: "abbreviated vendor and invoice punctuation",
            left: ObligationRef::new("Acme Industrial Supply", "1250.00", "USD")
                .with_invoice_number("INV-1044")
                .with_memo("Net-30 parts order for the March plant shutdown"),
            right: ObligationRef::new("Acme Ind. Supply", "1250.00", "USD")
                .with_invoice_number("1044")
                .with_memo("Parts for March shutdown, terms net 30"),
        },
        BaselineCase {
            id: "recurring-rent",
            label: "weekly rent, same vendor and amount, distinct bill ids",
            left: ObligationRef::new("Granite Bay Commercial Properties", "500.00", "USD")
                .with_vendor_id("vend-landlord")
                .with_bill_id("bill-operating-00003")
                .with_invoice_number("BILL-5002")
                .with_txn_date("2025-10-13")
                .with_memo("Stock received into the warehouse and counted."),
            right: ObligationRef::new("Granite Bay Commercial Properties", "500.00", "USD")
                .with_vendor_id("vend-landlord")
                .with_bill_id("bill-operating-00018")
                .with_invoice_number("BILL-5004")
                .with_txn_date("2025-10-20")
                .with_memo("Monthly service charge as agreed."),
        },
        BaselineCase {
            id: "different-bills",
            label: "two bills, different vendor and amount",
            left: ObligationRef::new("Ironclad Larkspur Outfitters", "4098.00", "USD")
                .with_vendor_id("vend-supplier")
                .with_bill_id("bill-00005")
                .with_invoice_number("BILL-5001")
                .with_txn_date("2025-10-10")
                .with_memo("Partial backorder noted; balance to follow separately."),
            right: ObligationRef::new("Granite Bay Commercial Properties", "500.00", "USD")
                .with_vendor_id("vend-landlord")
                .with_bill_id("bill-operating-00003")
                .with_invoice_number("BILL-5002")
                .with_txn_date("2025-10-13")
                .with_memo("Stock received into the warehouse and counted."),
        },
    ]
}

/// Compares the three cases using stored Jev response bodies, in case order.
///
/// Each body is mapped with the fixed thresholds. The resolver still skips
/// Jev when a guard decides, so a stored body does not override a guard.
#[must_use]
pub fn baseline_report(responses: [&str; 3]) -> BaselineReport {
    let cases = baseline_cases();
    let compared = cases
        .into_iter()
        .zip(responses)
        .map(|(case, response)| compare(&case, response))
        .collect();
    BaselineReport { cases: compared }
}

fn compare(case: &BaselineCase, response: &str) -> CaseComparison {
    let jev = map_binding_body(response);
    let resolution = resolve(&case.left, &case.right, Some(&Reply(response)));
    CaseComparison {
        id: case.id.to_owned(),
        label: case.label.to_owned(),
        exact: resolution.exact,
        fuzzy: resolution.fuzzy,
        jev,
        resolver: resolution.verdict,
        guard: resolution.guard,
        effect: resolution.effect,
        jev_changed_finding: changes_finding(jev, resolution.exact, resolution.fuzzy),
        fuzzy_would_merge: resolution.fuzzy == BindingVerdict::Same,
        resolver_called_jev: resolution.called_jev,
        jev_request: binding_request(&case.left, &case.right),
        jev_response: response.to_owned(),
    }
}

fn changes_finding(jev: BindingVerdict, exact: BindingVerdict, fuzzy: BindingVerdict) -> bool {
    !matches!(jev, BindingVerdict::Unknown) && jev != exact && jev != fuzzy
}

struct Reply<'a>(&'a str);

impl SystemOne for Reply<'_> {
    fn evaluate(&self, _request: &str) -> Result<String, JevCallFailure> {
        Ok(self.0.to_owned())
    }
}

/// A week of company-001-shaped rent. Distinct bill ids, one vendor, one amount.
#[cfg(test)]
fn weekly_rent(week: u32) -> ObligationRef {
    ObligationRef::new("Granite Bay Commercial Properties", "500.00", "USD")
        .with_vendor_id("vend-landlord")
        .with_bill_id(format!("bill-operating-{week:05}"))
        .with_invoice_number(format!("BILL-{}", 5000 + week))
        .with_memo("Monthly service charge as agreed.")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Boom;

    impl SystemOne for Boom {
        fn evaluate(&self, _request: &str) -> Result<String, JevCallFailure> {
            panic!("guards skip jev for distinct bill ids");
        }
    }

    #[test]
    fn fifty_two_weekly_rent_bills_do_not_merge() {
        let bills: Vec<_> = (1..=52).map(weekly_rent).collect();
        let mut merges = 0;
        for left in 0..bills.len() {
            for right in (left + 1)..bills.len() {
                let resolution = resolve(&bills[left], &bills[right], Some(&Boom));
                assert_eq!(resolution.verdict, BindingVerdict::Different);
                if resolution.effect.merged() {
                    merges += 1;
                }
            }
        }
        assert_eq!(merges, 0);
        assert_eq!(
            crate::fuzzy_match(&bills[0], &bills[1]),
            BindingVerdict::Same
        );
    }
}
