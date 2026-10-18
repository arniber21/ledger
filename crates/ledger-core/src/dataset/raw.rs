//! YAML shapes for company-001 books. Fields match `SCHEMA.md` for the three loaded entities.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawVendor {
    pub id: String,
    pub display_name: String,
    pub company_name: String,
    pub contact_given_name: String,
    pub contact_family_name: String,
    pub email: String,
    pub phone: String,
    pub address: RawAddress,
    pub payment_terms: String,
    pub payment_terms_days: u32,
    pub supply_role: String,
    pub joined_sim_day: u32,
    pub joined_date: String,
    pub sim_actor_id: String,
    pub sim_event_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawAddress {
    pub line1: String,
    pub line2: Option<String>,
    pub city: String,
    pub state: String,
    pub postal_code: String,
    pub country: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawBill {
    pub id: String,
    pub doc_number: String,
    pub vendor_id: String,
    pub vendor_name: String,
    pub expense_kind: String,
    pub txn_sim_day: u32,
    pub txn_date: String,
    pub due_sim_day: u32,
    pub due_date: String,
    pub item_lines: Vec<RawItemLine>,
    pub account_lines: Vec<RawAccountLine>,
    pub total_amount: String,
    pub project_id: Option<String>,
    pub cost_category: String,
    pub memo: String,
    pub provenance_note: String,
    pub sim_obligation_id: String,
    pub sim_order_id: Option<String>,
    pub sim_event_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawItemLine {
    pub line_num: u32,
    pub item_id: String,
    pub item_name: String,
    pub description: String,
    pub quantity: u32,
    pub unit_price_amount: String,
    pub amount: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawAccountLine {
    pub line_num: u32,
    pub account_id: String,
    pub account_name: String,
    pub description: String,
    pub amount: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawBillPayment {
    pub id: String,
    pub vendor_id: String,
    pub vendor_name: String,
    pub txn_sim_day: u32,
    pub txn_date: String,
    pub amount: String,
    pub applications: Vec<RawApplication>,
    pub payment_account_id: String,
    pub memo: String,
    pub provenance_note: String,
    pub sim_payment_id: String,
    pub sim_obligation_id: String,
    pub sim_event_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawApplication {
    pub document_id: String,
    pub amount: String,
}
