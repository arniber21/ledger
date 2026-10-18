//! Recurring schedule versus a duplicate payment.
//!
//! Same bill id and same period decide before Jev and skip the model.
//! Otherwise Jev is asked whether the new payment fits the vendor's recent
//! bill and payment history. The Choice is mapped with
//! [`crate::map_choice_body`] and the fixed 0.80 thresholds. `Same` means the
//! payment repeats an earlier bill and needs review. `Different` means it is
//! the next installment. [`BindingVerdict::Unknown`](crate::BindingVerdict::Unknown)
//! is inconclusive and is not a pass.
//!
//! `period_date` is the bill's transaction date. A gap of seven days is the
//! next weekly period. `paid_on` is the day money moved and does not decide
//! the period, so a payment a few days after its bill does not collapse the
//! next week's rent into the previous one.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use serde_json::{Map, Value};

use crate::matchers::{exact_match, fuzzy_match};
use crate::obligation::{filled, trimmed, ObligationRef};
use crate::resolve::CallFault;
use crate::{map_choice_body, BindingVerdict, JevCallFailure, SystemOne};

/// How many history rows the System One request keeps.
///
/// Guards still see the full history. The model sees the latest rows.
const RECENT_HISTORY: usize = 8;

const WEEK_DAYS: i64 = 7;

const LANDLORD_VENDOR: &str = "vend-landlord";
const LANDLORD_NAME: &str = "Granite Bay Commercial Properties";
const RENT_AMOUNT: &str = "500.00";
const RENT_CURRENCY: &str = "USD";
const PAYMENT_MEMO: &str = "Payment sent against vendor bill.";

/// Question id for the schedule Choice.
///
/// The id is for this crate. The model sees [`schedule_request`]'s
/// instructions and criteria, not this string.
pub const RECURRING_OR_DUPLICATE_QUESTION: &str = "recurring_or_duplicate";

/// A bill or a payment in a vendor history.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRecord {
    /// Bill row or payment row.
    pub kind: RecordKind,
    /// Stable bill id. Empty means the id is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bill_id: Option<String>,
    /// Books vendor id, when the row carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_id: Option<String>,
    /// Vendor name as written.
    pub vendor_name: String,
    /// Decimal-string amount, such as `"500.00"`.
    pub amount: String,
    /// Currency code, such as `"USD"`.
    pub currency: String,
    /// Bill date `YYYY-MM-DD`. This anchors the period.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_date: Option<String>,
    /// Day the payment moved, when this row is a payment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paid_on: Option<String>,
    /// Advisory invoice number. Not a stable id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice_number: Option<String>,
    /// Memo text. Wording is not an identity guard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

/// Whether a history row is a bill or a payment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordKind {
    /// A bill the company owes.
    Bill,
    /// Money applied to a bill.
    Payment,
}

impl ScheduleRecord {
    /// A bill with a vendor name, amount, and currency.
    #[must_use]
    pub fn bill(
        vendor_name: impl Into<String>,
        amount: impl Into<String>,
        currency: impl Into<String>,
    ) -> Self {
        Self::new(RecordKind::Bill, vendor_name, amount, currency)
    }

    /// A payment with a vendor name, amount, and currency.
    #[must_use]
    pub fn payment(
        vendor_name: impl Into<String>,
        amount: impl Into<String>,
        currency: impl Into<String>,
    ) -> Self {
        Self::new(RecordKind::Payment, vendor_name, amount, currency)
    }

    fn new(
        kind: RecordKind,
        vendor_name: impl Into<String>,
        amount: impl Into<String>,
        currency: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            bill_id: None,
            vendor_id: None,
            vendor_name: vendor_name.into(),
            amount: amount.into(),
            currency: currency.into(),
            period_date: None,
            paid_on: None,
            invoice_number: None,
            memo: None,
        }
    }

    /// Sets the stable bill id.
    #[must_use]
    pub fn with_bill_id(mut self, bill_id: impl Into<String>) -> Self {
        self.bill_id = Some(bill_id.into());
        self
    }

    /// Sets the books vendor id.
    #[must_use]
    pub fn with_vendor_id(mut self, vendor_id: impl Into<String>) -> Self {
        self.vendor_id = Some(vendor_id.into());
        self
    }

    /// Sets the bill date that anchors the period.
    #[must_use]
    pub fn with_period_date(mut self, period_date: impl Into<String>) -> Self {
        self.period_date = Some(period_date.into());
        self
    }

    /// Sets the day money moved.
    #[must_use]
    pub fn with_paid_on(mut self, paid_on: impl Into<String>) -> Self {
        self.paid_on = Some(paid_on.into());
        self
    }

    /// Sets the advisory invoice number.
    #[must_use]
    pub fn with_invoice_number(mut self, invoice_number: impl Into<String>) -> Self {
        self.invoice_number = Some(invoice_number.into());
        self
    }

    /// Sets the memo.
    #[must_use]
    pub fn with_memo(mut self, memo: impl Into<String>) -> Self {
        self.memo = Some(memo.into());
        self
    }
}

/// What the guards decided before Jev.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[must_use]
pub enum ScheduleGuard {
    /// Same bill id, or a date inside a period that already has a payment.
    Duplicate,
    /// A distinct bill on a later period of the same vendor and amount.
    Recurring,
    /// The guards cannot decide. Jev may run.
    Undecided,
}

/// What a reviewer should see for this payment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[must_use]
pub enum ScheduleFinding {
    /// The payment is the next installment. No violation.
    #[serde(rename = "recurring")]
    Recurring,
    /// The payment repeats an earlier bill.
    #[serde(rename = "needs review")]
    NeedsReview,
    /// Timeout, error, low score, contradiction, or missing evidence.
    ///
    /// Not a pass and not a violation.
    #[serde(rename = "inconclusive")]
    Inconclusive,
}

impl ScheduleFinding {
    /// Short label stored with the verdict. Never "fraud".
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Recurring => "recurring",
            Self::NeedsReview => "needs review",
            Self::Inconclusive => "inconclusive",
        }
    }
}

/// One schedule decision, including the exact and fuzzy baselines.
///
/// `exact` and `fuzzy` do not choose [`Self::finding`]. `jev_request` and
/// `jev_response` are present only when Jev ran.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleJudgment {
    /// Guard decision, or the mapped Choice when the guards were undecided.
    ///
    /// `Same` repeats an earlier bill. `Different` fits the schedule.
    /// `Unknown` is inconclusive.
    pub verdict: BindingVerdict,
    /// Recurring, needs review, or inconclusive.
    pub finding: ScheduleFinding,
    /// What the guards decided before Jev.
    pub guard: ScheduleGuard,
    /// Exact-match baseline against the latest same-vendor payment.
    pub exact: BindingVerdict,
    /// Fuzzy baseline. It ignores bill id and date.
    pub fuzzy: BindingVerdict,
    /// Whether [`judge_payment`] called Jev.
    pub called_jev: bool,
    /// Request body when Jev ran. No API key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_request: Option<String>,
    /// Response body when Jev returned one. Explanation fields are removed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_response: Option<String>,
    /// Why Jev produced no body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jev_fault: Option<CallFault>,
    /// Payment under review.
    pub candidate: ScheduleRecord,
}

impl ScheduleJudgment {
    /// JSON record for the verdict detail. No model prose is added.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("schedule judgment is finite JSON")
    }
}

/// One controlled payment, with the history the guards and Jev both see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleCase {
    /// Stable case id.
    pub id: &'static str,
    /// What the payment is for.
    pub label: &'static str,
    /// New payment.
    pub candidate: ScheduleRecord,
    /// Prior bills and payments. Does not include `candidate`.
    pub history: Vec<ScheduleRecord>,
}

/// Exact, fuzzy, Jev, and the resolver for one payment case.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleComparison {
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
    pub guard: ScheduleGuard,
    /// Recurring, needs review, or inconclusive.
    pub finding: ScheduleFinding,
    /// Jev's verdict differs from both baselines, and is not `Unknown`.
    pub jev_changed_finding: bool,
    /// Fuzzy returned `Same` for this pair. That is the naive duplicate flag.
    pub fuzzy_would_flag: bool,
    /// The resolver called Jev. False when a guard decided.
    pub resolver_called_jev: bool,
    /// Stored request body.
    pub jev_request: String,
    /// Stored response body.
    pub jev_response: String,
}

/// The recurring case and the in-period duplicate, in report order.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleReport {
    cases: Vec<ScheduleComparison>,
}

impl ScheduleReport {
    /// Cases in report order.
    #[must_use]
    pub fn cases(&self) -> &[ScheduleComparison] {
        &self.cases
    }

    /// JSON for the verdict detail. Cases only: there is no accuracy field.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("schedule report is finite JSON")
    }
}

/// Weekly rent's next installment, then a second payment of the same bill.
///
/// The rent ids are company-001 bills `bill-operating-00003` (2025-10-13)
/// and `bill-operating-00018` (2025-10-20), Granite Bay Commercial Properties,
/// 500.00 USD. The duplicate pays `bill-operating-00003` again inside that week.
#[must_use]
pub fn schedule_cases() -> [ScheduleCase; 2] {
    let first_bill = rent_bill(
        "bill-operating-00003",
        "BILL-5002",
        "2025-10-13",
        "Stock received into the warehouse and counted.",
    );
    let first_payment = rent_payment(
        "bill-operating-00003",
        "BILL-5002",
        "2025-10-13",
        "2025-10-16",
    );
    let next_bill = rent_bill(
        "bill-operating-00018",
        "BILL-5004",
        "2025-10-20",
        "Monthly service charge as agreed.",
    );
    let next_payment = rent_payment(
        "bill-operating-00018",
        "BILL-5004",
        "2025-10-20",
        "2025-10-21",
    );
    let second_payment = rent_payment(
        "bill-operating-00003",
        "BILL-5002",
        "2025-10-13",
        "2025-10-17",
    );
    [
        ScheduleCase {
            id: "recurring-schedule",
            label: "weekly rent, next period, distinct bill id",
            candidate: next_payment,
            history: vec![first_bill.clone(), first_payment.clone(), next_bill],
        },
        ScheduleCase {
            id: "duplicate-period",
            label: "second payment of the same bill inside one period",
            candidate: second_payment,
            history: vec![first_bill, first_payment],
        },
    ]
}

/// Compares the two payment cases using stored Jev response bodies, in case order.
///
/// Each body is mapped with the fixed thresholds. The resolver still skips
/// Jev when a guard decides, so a stored body does not override a guard.
#[must_use]
pub fn schedule_report(responses: [&str; 2]) -> ScheduleReport {
    let cases = schedule_cases();
    let compared = cases
        .into_iter()
        .zip(responses)
        .map(|(case, response)| compare(&case, response))
        .collect();
    ScheduleReport { cases: compared }
}

/// Judges `candidate` against `history`.
///
/// `history` is earlier bills and payments. It does not include `candidate`.
/// `jev` is called only when the guards are [`ScheduleGuard::Undecided`].
/// A missing client, a [`JevCallFailure`], or a low score becomes
/// [`ScheduleFinding::Inconclusive`].
#[must_use = "the judgment holds the finding"]
pub fn judge_payment(
    candidate: &ScheduleRecord,
    history: &[ScheduleRecord],
    jev: Option<&dyn SystemOne>,
) -> ScheduleJudgment {
    let (exact, fuzzy) = baselines(candidate, history);
    match schedule_guard(candidate, history) {
        ScheduleGuard::Duplicate => decided(
            candidate,
            ScheduleGuard::Duplicate,
            BindingVerdict::Same,
            exact,
            fuzzy,
        ),
        ScheduleGuard::Recurring => decided(
            candidate,
            ScheduleGuard::Recurring,
            BindingVerdict::Different,
            exact,
            fuzzy,
        ),
        ScheduleGuard::Undecided => ask(candidate, history, exact, fuzzy, jev),
    }
}

/// Body of `POST https://api.typesafe.ai/v1/systemone` for one payment.
///
/// The body has no API key. History is the latest [`RECENT_HISTORY`] rows.
/// Object keys are sorted so the saved fixture does not change when another
/// crate turns on `serde_json`'s `preserve_order`.
#[must_use]
pub fn schedule_request(candidate: &ScheduleRecord, history: &[ScheduleRecord]) -> String {
    let history_state: Vec<Value> = recent(history).into_iter().map(record_state).collect();
    let value = sort_objects(serde_json::json!({
        "state": {
            "candidate": record_state(candidate),
            "history": history_state
        },
        "model": "jev-latest",
        "questions": {
            RECURRING_OR_DUPLICATE_QUESTION: {
                "type": "choice",
                "instructions": "Does `candidate` fit the recurring schedule in `history`, or does it repeat an earlier bill?",
                "criteria": {
                    "same": {
                        "what": "The payment repeats an earlier bill.",
                        "still_same": "A second payment of a bill id that history already paid, or another payment whose bill date falls inside a period that already has a payment, repeats that bill.",
                        "not_for": "Do not pick this for a new bill id on a later period of the same vendor and amount. Memo wording is not a repeat."
                    },
                    "different": {
                        "what": "The payment is the next installment of the recurring schedule.",
                        "still_different": "A new bill id, a later period, and the same vendor and amount is the schedule, not a repeat.",
                        "not_for": "Do not pick this when the bill id was already paid or the bill date falls inside a period that already has a payment."
                    }
                }
            }
        }
    }));
    serde_json::to_string(&value).expect("schedule request is finite JSON")
}

fn schedule_guard(candidate: &ScheduleRecord, history: &[ScheduleRecord]) -> ScheduleGuard {
    if paid_bill(candidate, history) || in_paid_period(candidate, history) {
        return ScheduleGuard::Duplicate;
    }
    if new_installment(candidate, history) {
        return ScheduleGuard::Recurring;
    }
    ScheduleGuard::Undecided
}

fn paid_bill(candidate: &ScheduleRecord, history: &[ScheduleRecord]) -> bool {
    let Some(bill_id) = filled(&candidate.bill_id) else {
        return false;
    };
    history
        .iter()
        .any(|row| row.kind == RecordKind::Payment && filled(&row.bill_id) == Some(bill_id))
}

fn in_paid_period(candidate: &ScheduleRecord, history: &[ScheduleRecord]) -> bool {
    let Some(candidate_date) = date_of(&candidate.period_date) else {
        return false;
    };
    history.iter().any(|row| {
        same_schedule(candidate, row)
            && date_of(&row.period_date).is_some_and(|prior| within_week(candidate_date, prior))
    })
}

fn new_installment(candidate: &ScheduleRecord, history: &[ScheduleRecord]) -> bool {
    if filled(&candidate.bill_id).is_none() || !schedule_identity(candidate) {
        return false;
    }
    let Some(candidate_date) = date_of(&candidate.period_date) else {
        return false;
    };
    history.iter().all(|row| {
        if !same_schedule(candidate, row) {
            return true;
        }
        date_of(&row.period_date)
            .and_then(|prior| week_apart(candidate_date, prior))
            .unwrap_or(false)
    })
}

fn schedule_identity(record: &ScheduleRecord) -> bool {
    filled(&record.vendor_id).is_some()
        && parse_amount(&record.amount).is_some()
        && nonempty(&record.currency).is_some()
}

fn same_schedule(candidate: &ScheduleRecord, prior: &ScheduleRecord) -> bool {
    prior.kind == RecordKind::Payment
        && same_vendor(candidate, prior)
        && amounts_equal(&candidate.amount, &prior.amount)
        && currencies_equal(&candidate.currency, &prior.currency)
}

fn same_vendor(left: &ScheduleRecord, right: &ScheduleRecord) -> bool {
    match (filled(&left.vendor_id), filled(&right.vendor_id)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn decided(
    candidate: &ScheduleRecord,
    guard: ScheduleGuard,
    verdict: BindingVerdict,
    exact: BindingVerdict,
    fuzzy: BindingVerdict,
) -> ScheduleJudgment {
    ScheduleJudgment {
        verdict,
        finding: finding_of(verdict),
        guard,
        exact,
        fuzzy,
        called_jev: false,
        jev_request: None,
        jev_response: None,
        jev_fault: None,
        candidate: candidate.clone(),
    }
}

fn ask(
    candidate: &ScheduleRecord,
    history: &[ScheduleRecord],
    exact: BindingVerdict,
    fuzzy: BindingVerdict,
    jev: Option<&dyn SystemOne>,
) -> ScheduleJudgment {
    let Some(client) = jev else {
        return inconclusive(candidate, exact, fuzzy, None, None);
    };
    let request = schedule_request(candidate, history);
    match client.evaluate(&request) {
        Ok(body) => {
            let body = without_explanation(&body);
            let verdict = map_choice_body(&body, RECURRING_OR_DUPLICATE_QUESTION);
            ScheduleJudgment {
                verdict,
                finding: finding_of(verdict),
                guard: ScheduleGuard::Undecided,
                exact,
                fuzzy,
                called_jev: true,
                jev_request: Some(request),
                jev_response: Some(body),
                jev_fault: None,
                candidate: candidate.clone(),
            }
        }
        Err(failure) => inconclusive(candidate, exact, fuzzy, Some(request), Some(failure)),
    }
}

fn inconclusive(
    candidate: &ScheduleRecord,
    exact: BindingVerdict,
    fuzzy: BindingVerdict,
    request: Option<String>,
    failure: Option<JevCallFailure>,
) -> ScheduleJudgment {
    ScheduleJudgment {
        verdict: BindingVerdict::Unknown,
        finding: ScheduleFinding::Inconclusive,
        guard: ScheduleGuard::Undecided,
        exact,
        fuzzy,
        called_jev: request.is_some(),
        jev_request: request,
        jev_response: None,
        jev_fault: failure.map(CallFault::from),
        candidate: candidate.clone(),
    }
}

fn finding_of(verdict: BindingVerdict) -> ScheduleFinding {
    match verdict {
        BindingVerdict::Same => ScheduleFinding::NeedsReview,
        BindingVerdict::Different => ScheduleFinding::Recurring,
        BindingVerdict::Unknown => ScheduleFinding::Inconclusive,
    }
}

fn compare(case: &ScheduleCase, response: &str) -> ScheduleComparison {
    let judgment = judge_payment(&case.candidate, &case.history, Some(&Reply(response)));
    let jev = map_choice_body(response, RECURRING_OR_DUPLICATE_QUESTION);
    ScheduleComparison {
        id: case.id.to_owned(),
        label: case.label.to_owned(),
        exact: judgment.exact,
        fuzzy: judgment.fuzzy,
        jev,
        resolver: judgment.verdict,
        guard: judgment.guard,
        finding: judgment.finding,
        jev_changed_finding: changes_finding(jev, judgment.exact, judgment.fuzzy),
        fuzzy_would_flag: judgment.fuzzy == BindingVerdict::Same,
        resolver_called_jev: judgment.called_jev,
        jev_request: schedule_request(&case.candidate, &case.history),
        jev_response: response.to_owned(),
    }
}

fn changes_finding(jev: BindingVerdict, exact: BindingVerdict, fuzzy: BindingVerdict) -> bool {
    !matches!(jev, BindingVerdict::Unknown) && jev != exact && jev != fuzzy
}

fn baselines(
    candidate: &ScheduleRecord,
    history: &[ScheduleRecord],
) -> (BindingVerdict, BindingVerdict) {
    let Some(prior) = history
        .iter()
        .rev()
        .find(|row| row.kind == RecordKind::Payment && same_vendor(candidate, row))
    else {
        return (BindingVerdict::Different, BindingVerdict::Different);
    };
    let left = to_obligation(prior);
    let right = to_obligation(candidate);
    (exact_match(&left, &right), fuzzy_match(&left, &right))
}

fn to_obligation(record: &ScheduleRecord) -> ObligationRef {
    let mut obligation = ObligationRef::new(
        record.vendor_name.as_str(),
        record.amount.as_str(),
        record.currency.as_str(),
    );
    if let Some(id) = filled(&record.vendor_id) {
        obligation = obligation.with_vendor_id(id);
    }
    if let Some(id) = filled(&record.bill_id) {
        obligation = obligation.with_bill_id(id);
    }
    if let Some(id) = filled(&record.invoice_number) {
        obligation = obligation.with_invoice_number(id);
    }
    if let Some(date) = filled(&record.period_date) {
        obligation = obligation.with_txn_date(date);
    }
    if let Some(memo) = filled(&record.memo) {
        obligation = obligation.with_memo(memo);
    }
    obligation
}

fn recent(history: &[ScheduleRecord]) -> Vec<&ScheduleRecord> {
    let mut rows: Vec<&ScheduleRecord> = history.iter().collect();
    rows.sort_by(|left, right| {
        date_of(&left.period_date)
            .cmp(&date_of(&right.period_date))
            .then(kind_rank(left.kind).cmp(&kind_rank(right.kind)))
            .then(filled(&left.bill_id).cmp(&filled(&right.bill_id)))
            .then(date_of(&left.paid_on).cmp(&date_of(&right.paid_on)))
    });
    let start = rows.len().saturating_sub(RECENT_HISTORY);
    rows[start..].to_vec()
}

fn kind_rank(kind: RecordKind) -> u8 {
    match kind {
        RecordKind::Bill => 0,
        RecordKind::Payment => 1,
    }
}

fn record_state(record: &ScheduleRecord) -> Value {
    let mut map = Map::new();
    map.insert(
        "record".to_owned(),
        Value::String(
            match record.kind {
                RecordKind::Bill => "bill",
                RecordKind::Payment => "payment",
            }
            .to_owned(),
        ),
    );
    insert_filled(&mut map, "bill_id", record.bill_id.as_deref());
    insert_filled(&mut map, "vendor_id", record.vendor_id.as_deref());
    map.insert(
        "vendor_name".to_owned(),
        Value::String(record.vendor_name.clone()),
    );
    map.insert("amount".to_owned(), Value::String(record.amount.clone()));
    map.insert(
        "currency".to_owned(),
        Value::String(record.currency.clone()),
    );
    insert_filled(&mut map, "period_date", record.period_date.as_deref());
    insert_filled(&mut map, "paid_on", record.paid_on.as_deref());
    insert_filled(&mut map, "invoice_number", record.invoice_number.as_deref());
    insert_filled(&mut map, "memo", record.memo.as_deref());
    Value::Object(map)
}

fn insert_filled(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    let Some(text) = value.map(str::trim).filter(|text| !text.is_empty()) else {
        return;
    };
    map.insert(key.to_owned(), Value::String(text.to_owned()));
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

fn without_explanation(body: &str) -> String {
    let Ok(mut value) = serde_json::from_str::<Value>(body) else {
        return body.to_owned();
    };
    strip_explanation(&mut value);
    serde_json::to_string(&value).unwrap_or_else(|_| body.to_owned())
}

fn strip_explanation(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, _| key != "explanation");
            for child in map.values_mut() {
                strip_explanation(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_explanation(item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn within_week(left: NaiveDate, right: NaiveDate) -> bool {
    left.signed_duration_since(right)
        .num_days()
        .checked_abs()
        .is_some_and(|days| days < WEEK_DAYS)
}

fn week_apart(left: NaiveDate, right: NaiveDate) -> Option<bool> {
    left.signed_duration_since(right)
        .num_days()
        .checked_abs()
        .map(|days| days >= WEEK_DAYS)
}

fn date_of(value: &Option<String>) -> Option<NaiveDate> {
    let text = value.as_deref()?;
    parse_date(text)
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(trimmed(value), "%Y-%m-%d").ok()
}

fn parse_amount(value: &str) -> Option<ledger_core::dataset::Money> {
    nonempty(value)?.parse().ok()
}

fn amounts_equal(left: &str, right: &str) -> bool {
    match (parse_amount(left), parse_amount(right)) {
        (Some(left), Some(right)) => left == right,
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

fn rent_bill(bill_id: &str, invoice: &str, period: &str, memo: &str) -> ScheduleRecord {
    ScheduleRecord::bill(LANDLORD_NAME, RENT_AMOUNT, RENT_CURRENCY)
        .with_vendor_id(LANDLORD_VENDOR)
        .with_bill_id(bill_id)
        .with_invoice_number(invoice)
        .with_period_date(period)
        .with_memo(memo)
}

fn rent_payment(bill_id: &str, invoice: &str, period: &str, paid_on: &str) -> ScheduleRecord {
    ScheduleRecord::payment(LANDLORD_NAME, RENT_AMOUNT, RENT_CURRENCY)
        .with_vendor_id(LANDLORD_VENDOR)
        .with_bill_id(bill_id)
        .with_invoice_number(invoice)
        .with_period_date(period)
        .with_paid_on(paid_on)
        .with_memo(PAYMENT_MEMO)
}

struct Reply<'a>(&'a str);

impl SystemOne for Reply<'_> {
    fn evaluate(&self, _request: &str) -> Result<String, JevCallFailure> {
        Ok(self.0.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::*;

    const STOCK_MEMO: &str = "Stock received into the warehouse and counted.";
    const SERVICE_MEMO: &str = "Monthly service charge as agreed.";

    struct Boom;

    impl SystemOne for Boom {
        fn evaluate(&self, _request: &str) -> Result<String, JevCallFailure> {
            panic!("guards skip jev when the bill id or the period decides");
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
                RECURRING_OR_DUPLICATE_QUESTION: {
                    "type": "choice",
                    "choice": label,
                    "probabilities": { "same": same, "different": different }
                }
            }
        })
        .to_string()
    }

    fn ambiguous() -> (ScheduleRecord, Vec<ScheduleRecord>) {
        let prior = rent_payment(
            "bill-operating-00003",
            "BILL-5002",
            "2025-10-13",
            "2025-10-16",
        );
        let candidate = ScheduleRecord::payment(LANDLORD_NAME, RENT_AMOUNT, RENT_CURRENCY)
            .with_vendor_id(LANDLORD_VENDOR);
        (candidate, vec![prior])
    }

    fn week_date(week: u32) -> String {
        let start = NaiveDate::from_ymd_opt(2025, 10, 13).expect("start");
        let offset = i64::from(week - 1) * WEEK_DAYS;
        let date = start
            .checked_add_signed(TimeDelta::days(offset))
            .expect("week date");
        date.format("%Y-%m-%d").to_string()
    }

    fn lag(week: u32) -> i64 {
        match week % 3 {
            1 => 3,
            2 => 1,
            _ => 2,
        }
    }

    fn paid_on(period: &str, days: i64) -> String {
        let date = parse_date(period)
            .expect("period")
            .checked_add_signed(TimeDelta::days(days));
        date.expect("paid on").format("%Y-%m-%d").to_string()
    }

    #[test]
    fn fifty_two_weekly_rent_payments_raise_no_violation() {
        let mut history = Vec::new();
        let mut reviews = 0;
        for week in 1..=52 {
            let period = week_date(week);
            let bill_id = format!("bill-operating-{week:05}");
            let invoice = format!("BILL-{}", 5000 + week);
            let memo = if week % 2 == 1 {
                STOCK_MEMO
            } else {
                SERVICE_MEMO
            };
            let bill = rent_bill(&bill_id, &invoice, &period, memo);
            let payment = rent_payment(&bill_id, &invoice, &period, &paid_on(&period, lag(week)));
            history.push(bill);
            let judgment = judge_payment(&payment, &history, Some(&Boom));
            assert_eq!(
                judgment.finding,
                ScheduleFinding::Recurring,
                "week {week} period {period}"
            );
            assert!(!judgment.called_jev, "week {week}");
            assert_ne!(judgment.finding, ScheduleFinding::NeedsReview);
            if week > 1 {
                assert_eq!(judgment.exact, BindingVerdict::Different, "week {week}");
                assert_eq!(judgment.fuzzy, BindingVerdict::Same, "week {week}");
            }
            if judgment.finding == ScheduleFinding::NeedsReview {
                reviews += 1;
            }
            if week == 52 {
                assert_eq!(period, "2026-10-05");
            }
            history.push(payment);
        }
        assert_eq!(reviews, 0);
        assert_eq!(history.len(), 104);

        let again = rent_payment(
            "bill-operating-00001",
            "BILL-5001",
            "2025-10-13",
            "2025-10-17",
        );
        let review = judge_payment(&again, &history, Some(&Boom));
        assert_eq!(review.finding, ScheduleFinding::NeedsReview);
        assert_eq!(review.finding.label(), "needs review");
        assert_eq!(review.guard, ScheduleGuard::Duplicate);
        assert!(!review.called_jev);
        let json = review.to_json();
        assert!(json.contains("needs review"));
        assert!(!json.to_lowercase().contains("fraud"));
    }

    #[test]
    fn second_payment_inside_the_period_needs_review() {
        let bill = rent_bill(
            "bill-operating-00003",
            "BILL-5002",
            "2025-10-13",
            STOCK_MEMO,
        );
        let first = rent_payment(
            "bill-operating-00003",
            "BILL-5002",
            "2025-10-13",
            "2025-10-16",
        );
        let second = rent_payment(
            "bill-operating-00003",
            "BILL-5002",
            "2025-10-13",
            "2025-10-17",
        );
        let judgment = judge_payment(&second, &[bill, first], Some(&Boom));
        assert_eq!(judgment.finding, ScheduleFinding::NeedsReview);
        assert_eq!(judgment.verdict, BindingVerdict::Same);
        assert_eq!(judgment.exact, BindingVerdict::Same);
        assert_eq!(judgment.fuzzy, BindingVerdict::Same);
        assert!(!judgment.called_jev);
        assert!(!judgment.to_json().to_lowercase().contains("fraud"));
    }

    #[test]
    fn six_day_gap_with_a_new_bill_id_is_the_same_period() {
        let first = rent_payment(
            "bill-operating-00003",
            "BILL-5002",
            "2025-10-13",
            "2025-10-15",
        );
        let candidate = rent_payment(
            "bill-operating-00099",
            "BILL-5099",
            "2025-10-19",
            "2025-10-19",
        );
        let judgment = judge_payment(&candidate, &[first], Some(&Boom));
        assert_eq!(judgment.guard, ScheduleGuard::Duplicate);
        assert_eq!(judgment.finding, ScheduleFinding::NeedsReview);
        assert_eq!(judgment.exact, BindingVerdict::Different);
    }

    #[test]
    fn seven_day_gap_is_the_next_period_even_when_payment_dates_are_closer() {
        let first = rent_payment(
            "bill-operating-00003",
            "BILL-5002",
            "2025-10-13",
            "2025-10-16",
        );
        let next = rent_payment(
            "bill-operating-00018",
            "BILL-5004",
            "2025-10-20",
            "2025-10-21",
        );
        let judgment = judge_payment(&next, &[first], Some(&Boom));
        assert_eq!(judgment.guard, ScheduleGuard::Recurring);
        assert_eq!(judgment.finding, ScheduleFinding::Recurring);
        assert_eq!(judgment.fuzzy, BindingVerdict::Same);
        assert_eq!(judgment.exact, BindingVerdict::Different);
        assert!(!judgment.called_jev);
    }

    #[test]
    fn timeout_is_inconclusive_and_not_a_pass() {
        let (candidate, history) = ambiguous();
        let judgment = judge_payment(&candidate, &history, Some(&Fail(JevCallFailure::Timeout)));
        assert_eq!(judgment.verdict, BindingVerdict::Unknown);
        assert_eq!(judgment.finding, ScheduleFinding::Inconclusive);
        assert_ne!(judgment.finding, ScheduleFinding::Recurring);
        assert_ne!(judgment.finding, ScheduleFinding::NeedsReview);
        assert!(judgment.called_jev);
        assert!(judgment.jev_response.is_none());
        assert_eq!(judgment.jev_fault, Some(CallFault::Timeout));
    }

    #[test]
    fn score_under_threshold_is_inconclusive() {
        let (candidate, history) = ambiguous();
        let judgment = judge_payment(
            &candidate,
            &history,
            Some(&Reply(&choice("same", 0.79, 0.21))),
        );
        assert_eq!(judgment.verdict, BindingVerdict::Unknown);
        assert_eq!(judgment.finding, ScheduleFinding::Inconclusive);
        assert!(judgment.called_jev);
    }

    #[test]
    fn contradiction_is_inconclusive() {
        let (candidate, history) = ambiguous();
        let judgment = judge_payment(
            &candidate,
            &history,
            Some(&Reply(&choice("same", 0.90, 0.90))),
        );
        assert_eq!(judgment.finding, ScheduleFinding::Inconclusive);
        assert_ne!(judgment.finding, ScheduleFinding::Recurring);
    }

    #[test]
    fn missing_client_is_inconclusive() {
        let (candidate, history) = ambiguous();
        let judgment = judge_payment(&candidate, &history, None);
        assert_eq!(judgment.finding, ScheduleFinding::Inconclusive);
        assert!(!judgment.called_jev);
        assert!(judgment.jev_request.is_none());
    }

    #[test]
    fn mapped_same_needs_review_and_strips_explanation() {
        let (candidate, history) = ambiguous();
        let mut answer = serde_json::from_str::<Value>(&choice("same", 0.91, 0.09)).expect("json");
        answer["answers"][RECURRING_OR_DUPLICATE_QUESTION]["explanation"] =
            Value::String("model text that must not be kept".to_owned());
        let judgment = judge_payment(&candidate, &history, Some(&Reply(&answer.to_string())));
        assert_eq!(judgment.finding, ScheduleFinding::NeedsReview);
        let response = judgment.jev_response.as_deref().expect("body");
        assert!(!response.contains("explanation"));
        assert!(!judgment.to_json().to_lowercase().contains("explain"));
        assert!(!judgment.to_json().to_lowercase().contains("fraud"));
    }

    #[test]
    fn mapped_different_is_the_schedule() {
        let (candidate, history) = ambiguous();
        let judgment = judge_payment(
            &candidate,
            &history,
            Some(&Reply(&choice("different", 0.05, 0.95))),
        );
        assert_eq!(judgment.finding, ScheduleFinding::Recurring);
        assert_eq!(judgment.verdict, BindingVerdict::Different);
        assert!(judgment.called_jev);
    }

    #[test]
    fn request_carries_recent_history_and_asks_for_no_explanation() {
        let case = &schedule_cases()[0];
        let request = schedule_request(&case.candidate, &case.history);
        let value: Value = serde_json::from_str(&request).expect("request JSON");
        assert_eq!(value["model"], "jev-latest");
        let question = &value["questions"][RECURRING_OR_DUPLICATE_QUESTION];
        assert_eq!(question["type"], "choice");
        let criteria = question["criteria"].as_object().expect("criteria");
        assert!(criteria.contains_key("same"));
        assert!(criteria.contains_key("different"));
        let history = value["state"]["history"].as_array().expect("history");
        assert!(history.len() >= 2);
        assert_eq!(
            value["state"]["candidate"]["bill_id"],
            "bill-operating-00018"
        );
        assert!(value.get("explanation").is_none());
        assert!(!request.to_ascii_lowercase().contains("explain"));
        assert!(!request.to_lowercase().contains("fraud"));
        assert!(!request.contains("TYPESAFE_API_KEY"));
        assert!(!request.contains("Bearer"));
    }

    #[test]
    fn both_cases_are_guarded_before_jev() {
        let cases = schedule_cases();
        let recurring = judge_payment(&cases[0].candidate, &cases[0].history, Some(&Boom));
        assert_eq!(recurring.finding, ScheduleFinding::Recurring);
        assert_eq!(recurring.exact, BindingVerdict::Different);
        assert_eq!(recurring.fuzzy, BindingVerdict::Same);
        let duplicate = judge_payment(&cases[1].candidate, &cases[1].history, Some(&Boom));
        assert_eq!(duplicate.finding, ScheduleFinding::NeedsReview);
        assert_eq!(duplicate.exact, BindingVerdict::Same);
        assert_eq!(duplicate.fuzzy, BindingVerdict::Same);
        assert_eq!(duplicate.finding.label(), "needs review");
    }
}
