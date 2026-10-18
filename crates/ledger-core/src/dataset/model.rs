//! Typed bills, vendors, and bill payments for one company.

use chrono::NaiveDate;

use crate::dataset::ids::{AccountId, BillId, BillPaymentId, ItemId, ProjectId, VendorId};
use crate::dataset::money::Money;

/// What a bill purchased.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExpenseKind {
    /// Goods delivered into stock.
    Product,
    /// Overhead, including rent.
    Operating,
}

impl ExpenseKind {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "product" => Some(Self::Product),
            "operating" => Some(Self::Operating),
            _ => None,
        }
    }
}

/// What a vendor supplies. Values are the dataset's `supply_role` vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SupplyRole {
    /// Goods the business resells.
    Product,
    /// Trade labor.
    Subcontract,
    /// Rent.
    Premises,
    /// Utility service.
    Utility,
    /// Insurance.
    Insurance,
    /// Interest.
    Lender,
    /// Permits and tax.
    Government,
    /// Other overhead services.
    Operating,
}

impl SupplyRole {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "product" => Some(Self::Product),
            "subcontract" => Some(Self::Subcontract),
            "premises" => Some(Self::Premises),
            "utility" => Some(Self::Utility),
            "insurance" => Some(Self::Insurance),
            "lender" => Some(Self::Lender),
            "government" => Some(Self::Government),
            "operating" => Some(Self::Operating),
            _ => None,
        }
    }
}

/// Remittance or business address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub line1: String,
    pub line2: Option<String>,
    pub city: String,
    pub state: String,
    pub postal_code: String,
    pub country: String,
}

/// A vendor the company buys from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vendor {
    pub id: VendorId,
    pub display_name: String,
    pub company_name: String,
    pub contact_given_name: String,
    pub contact_family_name: String,
    pub email: String,
    pub phone: String,
    pub address: Address,
    pub payment_terms: String,
    pub payment_terms_days: u32,
    pub supply_role: SupplyRole,
    pub joined_sim_day: u32,
    pub joined_date: NaiveDate,
    pub sim_actor_id: String,
    pub sim_event_ids: Vec<String>,
}

/// One goods line on a product bill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemLine {
    pub line_num: u32,
    pub item_id: ItemId,
    pub item_name: String,
    pub description: String,
    pub quantity: u32,
    pub unit_price_amount: Money,
    pub amount: Money,
}

/// One account charge on an overhead bill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountLine {
    pub line_num: u32,
    pub account_id: AccountId,
    pub account_name: String,
    pub description: String,
    pub amount: Money,
}

/// A purchase received on credit.
///
/// `doc_number` is advisory. Providers renumber documents, so callers key on [`Bill::id`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bill {
    pub id: BillId,
    pub doc_number: String,
    pub vendor_id: VendorId,
    pub vendor_name: String,
    pub expense_kind: ExpenseKind,
    pub txn_sim_day: u32,
    pub txn_date: NaiveDate,
    pub due_sim_day: u32,
    pub due_date: NaiveDate,
    pub item_lines: Vec<ItemLine>,
    pub account_lines: Vec<AccountLine>,
    pub total_amount: Money,
    pub project_id: Option<ProjectId>,
    pub cost_category: String,
    pub memo: String,
    pub provenance_note: String,
    pub sim_obligation_id: String,
    pub sim_order_id: Option<String>,
    pub sim_event_ids: Vec<String>,
}

/// One bill settled by a [`BillPayment`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentApplication {
    pub document_id: BillId,
    pub amount: Money,
}

/// Money paid to a vendor against their bills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillPayment {
    pub id: BillPaymentId,
    pub vendor_id: VendorId,
    pub vendor_name: String,
    pub txn_sim_day: u32,
    pub txn_date: NaiveDate,
    pub amount: Money,
    pub applications: Vec<PaymentApplication>,
    pub payment_account_id: AccountId,
    pub memo: String,
    pub provenance_note: String,
    pub sim_payment_id: String,
    pub sim_obligation_id: String,
    pub sim_event_ids: Vec<String>,
}
