//! One bill or vendor reference compared for obligation identity.
//!
//! Fields are the short, structured record sent to Jev. Empty optional fields
//! are omitted from that record.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A bill, or a vendor reference that might name that bill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObligationRef {
    /// Books vendor id, when the reference carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_id: Option<String>,
    /// Vendor name as written on the reference.
    pub vendor_name: String,
    /// Stable bill id. Advisory document numbers are [`Self::invoice_number`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bill_id: Option<String>,
    /// Invoice or document number as written. Not a stable id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invoice_number: Option<String>,
    /// Decimal-string amount, such as `"500.00"`.
    pub amount: String,
    /// Currency code, such as `"USD"`.
    pub currency: String,
    /// Transaction date `YYYY-MM-DD`, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub txn_date: Option<String>,
    /// Memo text. Wording is not an identity guard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

impl ObligationRef {
    /// A reference with a vendor name, amount, and currency.
    #[must_use]
    pub fn new(
        vendor_name: impl Into<String>,
        amount: impl Into<String>,
        currency: impl Into<String>,
    ) -> Self {
        Self {
            vendor_id: None,
            vendor_name: vendor_name.into(),
            bill_id: None,
            invoice_number: None,
            amount: amount.into(),
            currency: currency.into(),
            txn_date: None,
            memo: None,
        }
    }

    /// Sets the books vendor id.
    #[must_use]
    pub fn with_vendor_id(mut self, vendor_id: impl Into<String>) -> Self {
        self.vendor_id = Some(vendor_id.into());
        self
    }

    /// Sets the stable bill id.
    #[must_use]
    pub fn with_bill_id(mut self, bill_id: impl Into<String>) -> Self {
        self.bill_id = Some(bill_id.into());
        self
    }

    /// Sets the advisory invoice number.
    #[must_use]
    pub fn with_invoice_number(mut self, invoice_number: impl Into<String>) -> Self {
        self.invoice_number = Some(invoice_number.into());
        self
    }

    /// Sets the transaction date.
    #[must_use]
    pub fn with_txn_date(mut self, txn_date: impl Into<String>) -> Self {
        self.txn_date = Some(txn_date.into());
        self
    }

    /// Sets the memo.
    #[must_use]
    pub fn with_memo(mut self, memo: impl Into<String>) -> Self {
        self.memo = Some(memo.into());
        self
    }
}

pub(crate) fn filled(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

pub(crate) fn trimmed(value: &str) -> &str {
    value.trim()
}

/// State object for one side of a binding request. Empty optionals are omitted.
pub(crate) fn ref_state(reference: &ObligationRef) -> Value {
    let mut map = Map::new();
    insert_filled(&mut map, "vendor_id", reference.vendor_id.as_deref());
    map.insert(
        "vendor_name".to_owned(),
        Value::String(reference.vendor_name.clone()),
    );
    insert_filled(&mut map, "bill_id", reference.bill_id.as_deref());
    insert_filled(
        &mut map,
        "invoice_number",
        reference.invoice_number.as_deref(),
    );
    map.insert("amount".to_owned(), Value::String(reference.amount.clone()));
    map.insert(
        "currency".to_owned(),
        Value::String(reference.currency.clone()),
    );
    insert_filled(&mut map, "txn_date", reference.txn_date.as_deref());
    insert_filled(&mut map, "memo", reference.memo.as_deref());
    Value::Object(map)
}

fn insert_filled(map: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    let Some(text) = value.map(str::trim).filter(|text| !text.is_empty()) else {
        return;
    };
    map.insert(key.to_owned(), Value::String(text.to_owned()));
}
