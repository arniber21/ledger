//! Loader for one company's bills, vendors, and bill payments.
//!
//! `just data` extracts company-001 to
//! `data/hackathon-100/company-001/rwm-v4/world-*/` as one YAML file per entity
//! type. This module reads `vendors.yaml`, `bills.yaml`, and `bill_payments.yaml`.
//! It does not read `hidden/` (simulation ground truth).

mod ids;
mod model;
mod money;
mod raw;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use chrono::NaiveDate;
use thiserror::Error;

pub use ids::{AccountId, BillId, BillPaymentId, CompanyId, IdError, ItemId, ProjectId, VendorId};
pub use model::{
    AccountLine, Address, Bill, BillPayment, ExpenseKind, ItemLine, PaymentApplication, SupplyRole,
    Vendor,
};
pub use money::{Money, MoneyParseError};

use ids::ScopedId;
use model::{AccountLine as ModelAccountLine, ItemLine as ModelItemLine};
use raw::{RawBill, RawBillPayment, RawVendor};

/// Books for one company: vendors, bills, and the payments applied to those bills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanyBooks {
    company_id: CompanyId,
    vendors: Vec<Vendor>,
    vendor_index: HashMap<VendorId, usize>,
    bills: Vec<Bill>,
    bill_index: HashMap<BillId, usize>,
    bill_payments: Vec<BillPayment>,
}

impl CompanyBooks {
    /// Company scope stamped onto every id in these books.
    #[must_use]
    pub fn company_id(&self) -> &CompanyId {
        &self.company_id
    }

    /// Vendors in file order.
    #[must_use]
    pub fn vendors(&self) -> &[Vendor] {
        &self.vendors
    }

    /// Bills in file order. Each bill has a vendor id, vendor name, and due date.
    #[must_use]
    pub fn bills(&self) -> &[Bill] {
        &self.bills
    }

    /// Bill payments in file order.
    #[must_use]
    pub fn bill_payments(&self) -> &[BillPayment] {
        &self.bill_payments
    }

    /// Vendor record for `id`, when this company has that vendor.
    #[must_use]
    pub fn vendor(&self, id: &VendorId) -> Option<&Vendor> {
        self.vendor_index.get(id).map(|index| &self.vendors[*index])
    }

    /// Bill record for `id`, when this company has that bill.
    #[must_use]
    pub fn bill(&self, id: &BillId) -> Option<&Bill> {
        self.bill_index.get(id).map(|index| &self.bills[*index])
    }

    /// Bills whose vendor is `vendor`, in file order.
    pub fn bills_for_vendor<'a>(
        &'a self,
        vendor: &'a VendorId,
    ) -> impl Iterator<Item = &'a Bill> + 'a {
        self.bills
            .iter()
            .filter(move |bill| &bill.vendor_id == vendor)
    }
}

/// Failure while loading a company directory.
#[derive(Debug, Error)]
pub enum LoadError {
    /// A path component is `hidden`. The loader does not read simulation ground truth.
    #[error("refusing to read a path under hidden: {0}")]
    Hidden(PathBuf),

    /// The path is not a `company-*` directory.
    #[error("not a company directory: {0}")]
    CompanyDir(PathBuf),

    /// A dataset file or directory could not be read.
    #[error("failed to read {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// YAML did not match the books schema.
    #[error("failed to parse {path}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },

    /// `rwm-v4/` contains no `world-*` directory.
    #[error("no world directory under {0}")]
    NoWorld(PathBuf),

    /// `rwm-v4/` contains more than one `world-*` directory.
    #[error("multiple world directories under {0}")]
    MultipleWorlds(PathBuf),

    /// A record failed a field, reference, or amount check.
    #[error("invalid {entity} {id}: {reason}")]
    Invalid {
        entity: &'static str,
        id: String,
        reason: String,
    },
}

/// Loads bills, vendors, and bill payments for the company at `company_dir`.
///
/// `company_dir` is a folder such as `data/hackathon-100/company-001`, containing
/// `rwm-v4/world-*`. The directory name becomes the [`CompanyId`] stamped on every id.
/// Money fields are decimal strings. The dataset is not modified.
///
/// # Errors
///
/// Returns [`LoadError::Hidden`] when any path component is `hidden`, without reading it.
/// Returns [`LoadError::CompanyDir`] when the folder is not named `company-*`.
/// Returns [`LoadError::Read`] or [`LoadError::Parse`] when a YAML file is missing or
/// does not match the schema. Returns [`LoadError::NoWorld`] or [`LoadError::MultipleWorlds`]
/// when `rwm-v4` does not contain exactly one world. Returns [`LoadError::Invalid`] when
/// an id, date, amount, vendor reference, or line total does not check out.
#[must_use = "loading the dataset has no side effects beyond the returned books"]
pub fn load_company(company_dir: &Path) -> Result<CompanyBooks, LoadError> {
    ensure_not_hidden(company_dir)?;
    let company_id = company_id_of(company_dir)?;
    let world = world_dir(company_dir)?;

    let vendors_path = world.join("vendors.yaml");
    let bills_path = world.join("bills.yaml");
    let payments_path = world.join("bill_payments.yaml");
    ensure_not_hidden(&vendors_path)?;
    ensure_not_hidden(&bills_path)?;
    ensure_not_hidden(&payments_path)?;

    let raw_vendors: Vec<RawVendor> = read_yaml(&vendors_path)?;
    let raw_bills: Vec<RawBill> = read_yaml(&bills_path)?;
    let raw_payments: Vec<RawBillPayment> = read_yaml(&payments_path)?;

    let (vendors, vendor_index) = build_vendors(&company_id, raw_vendors)?;
    let (bills, bill_index) = build_bills(&company_id, raw_bills, &vendors, &vendor_index)?;
    let bill_payments = build_payments(
        &company_id,
        raw_payments,
        &vendors,
        &vendor_index,
        &bills,
        &bill_index,
    )?;

    Ok(CompanyBooks {
        company_id,
        vendors,
        vendor_index,
        bills,
        bill_index,
        bill_payments,
    })
}

fn ensure_not_hidden(path: &Path) -> Result<(), LoadError> {
    let under_hidden = path
        .components()
        .any(|component| matches!(component, Component::Normal(name) if name == "hidden"));
    if under_hidden {
        Err(LoadError::Hidden(path.to_owned()))
    } else {
        Ok(())
    }
}

fn company_id_of(company_dir: &Path) -> Result<CompanyId, LoadError> {
    let Some(name) = company_dir.file_name().and_then(|name| name.to_str()) else {
        return Err(LoadError::CompanyDir(company_dir.to_owned()));
    };
    if !name.starts_with("company-") || name.len() == "company-".len() {
        return Err(LoadError::CompanyDir(company_dir.to_owned()));
    }
    CompanyId::new(name).map_err(|_| LoadError::CompanyDir(company_dir.to_owned()))
}

fn world_dir(company_dir: &Path) -> Result<PathBuf, LoadError> {
    let rwm = company_dir.join("rwm-v4");
    ensure_not_hidden(&rwm)?;
    let entries = fs::read_dir(&rwm).map_err(|source| LoadError::Read {
        path: rwm.clone(),
        source,
    })?;

    let mut worlds = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| LoadError::Read {
            path: rwm.clone(),
            source,
        })?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if !name.starts_with("world-") {
            continue;
        }
        let path = entry.path();
        ensure_not_hidden(&path)?;
        if path.is_dir() {
            worlds.push(path);
        }
    }

    match worlds.as_slice() {
        [only] => Ok(only.clone()),
        [] => Err(LoadError::NoWorld(rwm)),
        _ => Err(LoadError::MultipleWorlds(rwm)),
    }
}

fn read_yaml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, LoadError> {
    let file = fs::File::open(path).map_err(|source| LoadError::Read {
        path: path.to_owned(),
        source,
    })?;
    serde_yaml::from_reader(file).map_err(|source| LoadError::Parse {
        path: path.to_owned(),
        source,
    })
}

fn build_vendors(
    company_id: &CompanyId,
    raw_vendors: Vec<RawVendor>,
) -> Result<(Vec<Vendor>, HashMap<VendorId, usize>), LoadError> {
    let mut vendors = Vec::with_capacity(raw_vendors.len());
    let mut vendor_index = HashMap::with_capacity(raw_vendors.len());
    for raw in raw_vendors {
        let vendor = convert_vendor(company_id, raw)?;
        if vendor_index.contains_key(&vendor.id) {
            return Err(invalid("vendor", vendor.id.local(), "duplicate id"));
        }
        vendor_index.insert(vendor.id.clone(), vendors.len());
        vendors.push(vendor);
    }
    Ok((vendors, vendor_index))
}

fn build_bills(
    company_id: &CompanyId,
    raw_bills: Vec<RawBill>,
    vendors: &[Vendor],
    vendor_index: &HashMap<VendorId, usize>,
) -> Result<(Vec<Bill>, HashMap<BillId, usize>), LoadError> {
    let mut bills = Vec::with_capacity(raw_bills.len());
    let mut bill_index = HashMap::with_capacity(raw_bills.len());
    for raw in raw_bills {
        let bill = convert_bill(company_id, raw, vendors, vendor_index)?;
        if bill_index.contains_key(&bill.id) {
            return Err(invalid("bill", bill.id.local(), "duplicate id"));
        }
        bill_index.insert(bill.id.clone(), bills.len());
        bills.push(bill);
    }
    Ok((bills, bill_index))
}

fn build_payments(
    company_id: &CompanyId,
    raw_payments: Vec<RawBillPayment>,
    vendors: &[Vendor],
    vendor_index: &HashMap<VendorId, usize>,
    bills: &[Bill],
    bill_index: &HashMap<BillId, usize>,
) -> Result<Vec<BillPayment>, LoadError> {
    let mut payments = Vec::with_capacity(raw_payments.len());
    let mut seen = HashSet::with_capacity(raw_payments.len());
    for raw in raw_payments {
        let payment = convert_payment(company_id, raw, vendors, vendor_index, bills, bill_index)?;
        if !seen.insert(payment.id.clone()) {
            return Err(invalid("bill payment", payment.id.local(), "duplicate id"));
        }
        payments.push(payment);
    }
    Ok(payments)
}

fn convert_vendor(company_id: &CompanyId, raw: RawVendor) -> Result<Vendor, LoadError> {
    let id = vendor_id(company_id, &raw.id)?;
    let supply_role = SupplyRole::parse(&raw.supply_role).ok_or_else(|| {
        invalid(
            "vendor",
            &raw.id,
            format!("unknown supply_role {}", raw.supply_role),
        )
    })?;
    Ok(Vendor {
        id,
        display_name: raw.display_name,
        company_name: raw.company_name,
        contact_given_name: raw.contact_given_name,
        contact_family_name: raw.contact_family_name,
        email: raw.email,
        phone: raw.phone,
        address: Address {
            line1: raw.address.line1,
            line2: raw.address.line2,
            city: raw.address.city,
            state: raw.address.state,
            postal_code: raw.address.postal_code,
            country: raw.address.country,
        },
        payment_terms: raw.payment_terms,
        payment_terms_days: raw.payment_terms_days,
        supply_role,
        joined_sim_day: raw.joined_sim_day,
        joined_date: parse_date("vendor", &raw.id, "joined_date", &raw.joined_date)?,
        sim_actor_id: raw.sim_actor_id,
        sim_event_ids: raw.sim_event_ids,
    })
}

fn convert_bill(
    company_id: &CompanyId,
    raw: RawBill,
    vendors: &[Vendor],
    vendor_index: &HashMap<VendorId, usize>,
) -> Result<Bill, LoadError> {
    let id = bill_id(company_id, &raw.id)?;
    let vendor_id = vendor_id(company_id, &raw.vendor_id)?;
    let vendor = at_index(vendors, vendor_index.get(&vendor_id).copied())
        .ok_or_else(|| invalid("bill", &raw.id, format!("unknown vendor {}", raw.vendor_id)))?;
    if vendor.display_name != raw.vendor_name {
        return Err(invalid(
            "bill",
            &raw.id,
            format!(
                "vendor name {} does not match vendor {} ({})",
                raw.vendor_name, raw.vendor_id, vendor.display_name
            ),
        ));
    }

    let expense_kind = ExpenseKind::parse(&raw.expense_kind).ok_or_else(|| {
        invalid(
            "bill",
            &raw.id,
            format!("unknown expense_kind {}", raw.expense_kind),
        )
    })?;

    let mut item_lines = Vec::with_capacity(raw.item_lines.len());
    for line in raw.item_lines {
        let unit_price_amount = parse_money(
            "bill",
            &raw.id,
            "unit_price_amount",
            &line.unit_price_amount,
        )?;
        let amount = parse_money("bill", &raw.id, "amount", &line.amount)?;
        let extended = unit_price_amount
            .checked_mul_u32(line.quantity)
            .ok_or_else(|| invalid("bill", &raw.id, "item line amount overflow"))?;
        if extended != amount {
            return Err(invalid(
                "bill",
                &raw.id,
                format!(
                    "item line {} quantity times unit price is {extended}, line amount is {amount}",
                    line.line_num
                ),
            ));
        }
        item_lines.push(ModelItemLine {
            line_num: line.line_num,
            item_id: scoped(company_id, "bill", &raw.id, "item_id", &line.item_id)?,
            item_name: line.item_name,
            description: line.description,
            quantity: line.quantity,
            unit_price_amount,
            amount,
        });
    }

    let mut account_lines = Vec::with_capacity(raw.account_lines.len());
    for line in raw.account_lines {
        account_lines.push(ModelAccountLine {
            line_num: line.line_num,
            account_id: scoped(company_id, "bill", &raw.id, "account_id", &line.account_id)?,
            account_name: line.account_name,
            description: line.description,
            amount: parse_money("bill", &raw.id, "amount", &line.amount)?,
        });
    }

    let total_amount = parse_money("bill", &raw.id, "total_amount", &raw.total_amount)?;
    let mut line_amounts = Vec::with_capacity(item_lines.len() + account_lines.len());
    line_amounts.extend(item_lines.iter().map(|line| line.amount));
    line_amounts.extend(account_lines.iter().map(|line| line.amount));
    let sum = sum_amounts("bill", &raw.id, &line_amounts)?;
    if sum != total_amount {
        return Err(invalid(
            "bill",
            &raw.id,
            format!("line sum {sum} does not match total {total_amount}"),
        ));
    }

    Ok(Bill {
        id,
        doc_number: raw.doc_number,
        vendor_id,
        vendor_name: raw.vendor_name,
        expense_kind,
        txn_sim_day: raw.txn_sim_day,
        txn_date: parse_date("bill", &raw.id, "txn_date", &raw.txn_date)?,
        due_sim_day: raw.due_sim_day,
        due_date: parse_date("bill", &raw.id, "due_date", &raw.due_date)?,
        item_lines,
        account_lines,
        total_amount,
        project_id: optional_scoped(company_id, "bill", &raw.id, "project_id", raw.project_id)?,
        cost_category: raw.cost_category,
        memo: raw.memo,
        provenance_note: raw.provenance_note,
        sim_obligation_id: raw.sim_obligation_id,
        sim_order_id: raw.sim_order_id,
        sim_event_ids: raw.sim_event_ids,
    })
}

fn convert_payment(
    company_id: &CompanyId,
    raw: RawBillPayment,
    vendors: &[Vendor],
    vendor_index: &HashMap<VendorId, usize>,
    bills: &[Bill],
    bill_index: &HashMap<BillId, usize>,
) -> Result<BillPayment, LoadError> {
    let id = payment_id(company_id, &raw.id)?;
    let vendor_id = vendor_id(company_id, &raw.vendor_id)?;
    let vendor = at_index(vendors, vendor_index.get(&vendor_id).copied()).ok_or_else(|| {
        invalid(
            "bill payment",
            &raw.id,
            format!("unknown vendor {}", raw.vendor_id),
        )
    })?;
    if vendor.display_name != raw.vendor_name {
        return Err(invalid(
            "bill payment",
            &raw.id,
            format!(
                "vendor name {} does not match vendor {} ({})",
                raw.vendor_name, raw.vendor_id, vendor.display_name
            ),
        ));
    }

    let mut applications = Vec::with_capacity(raw.applications.len());
    for application in raw.applications {
        let document_id = bill_id(company_id, &application.document_id)?;
        let bill = at_index(bills, bill_index.get(&document_id).copied()).ok_or_else(|| {
            invalid(
                "bill payment",
                &raw.id,
                format!("unknown bill {}", application.document_id),
            )
        })?;
        if bill.vendor_id != vendor_id {
            return Err(invalid(
                "bill payment",
                &raw.id,
                format!(
                    "bill {} belongs to vendor {}",
                    application.document_id,
                    bill.vendor_id.local()
                ),
            ));
        }
        applications.push(PaymentApplication {
            document_id,
            amount: parse_money("bill payment", &raw.id, "amount", &application.amount)?,
        });
    }

    let amount = parse_money("bill payment", &raw.id, "amount", &raw.amount)?;
    let applied = sum_amounts(
        "bill payment",
        &raw.id,
        &applications
            .iter()
            .map(|application| application.amount)
            .collect::<Vec<_>>(),
    )?;
    if applied != amount {
        return Err(invalid(
            "bill payment",
            &raw.id,
            format!("applications sum to {applied}, payment amount is {amount}"),
        ));
    }

    Ok(BillPayment {
        id,
        vendor_id,
        vendor_name: raw.vendor_name,
        txn_sim_day: raw.txn_sim_day,
        txn_date: parse_date("bill payment", &raw.id, "txn_date", &raw.txn_date)?,
        amount,
        applications,
        payment_account_id: scoped(
            company_id,
            "bill payment",
            &raw.id,
            "payment_account_id",
            &raw.payment_account_id,
        )?,
        memo: raw.memo,
        provenance_note: raw.provenance_note,
        sim_payment_id: raw.sim_payment_id,
        sim_obligation_id: raw.sim_obligation_id,
        sim_event_ids: raw.sim_event_ids,
    })
}

fn vendor_id(company_id: &CompanyId, local: &str) -> Result<VendorId, LoadError> {
    scoped(company_id, "vendor", local, "id", local)
}

fn bill_id(company_id: &CompanyId, local: &str) -> Result<BillId, LoadError> {
    scoped(company_id, "bill", local, "id", local)
}

fn payment_id(company_id: &CompanyId, local: &str) -> Result<BillPaymentId, LoadError> {
    scoped(company_id, "bill payment", local, "id", local)
}

fn scoped<Kind>(
    company_id: &CompanyId,
    entity: &'static str,
    id: &str,
    field: &str,
    local: &str,
) -> Result<ScopedId<Kind>, LoadError> {
    ScopedId::new(company_id.clone(), local)
        .map_err(|_| invalid(entity, id, format!("invalid {field}")))
}

fn optional_scoped<Kind>(
    company_id: &CompanyId,
    entity: &'static str,
    id: &str,
    field: &str,
    local: Option<String>,
) -> Result<Option<ScopedId<Kind>>, LoadError> {
    local
        .as_deref()
        .map(|local| scoped(company_id, entity, id, field, local))
        .transpose()
}

fn parse_money(entity: &'static str, id: &str, field: &str, raw: &str) -> Result<Money, LoadError> {
    Money::from_str(raw)
        .map_err(|_| invalid(entity, id, format!("{field} is not a two-decimal string")))
}

fn parse_date(
    entity: &'static str,
    id: &str,
    field: &str,
    raw: &str,
) -> Result<NaiveDate, LoadError> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| invalid(entity, id, format!("{field} is not YYYY-MM-DD")))
}

fn sum_amounts(entity: &'static str, id: &str, amounts: &[Money]) -> Result<Money, LoadError> {
    let mut total = Money::ZERO;
    for amount in amounts {
        total = total
            .checked_add(*amount)
            .ok_or_else(|| invalid(entity, id, "amount overflow"))?;
    }
    Ok(total)
}

fn at_index<T>(items: &[T], index: Option<usize>) -> Option<&T> {
    items.get(index?)
}

fn invalid(entity: &'static str, id: &str, reason: impl Into<String>) -> LoadError {
    LoadError::Invalid {
        entity,
        id: id.to_owned(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::NaiveDate;
    use std::path::PathBuf;

    #[test]
    fn load_company_rejects_a_hidden_path_without_reading_it() {
        let path = Path::new("data/hackathon-100/company-001/hidden/world");
        let error = load_company(path).unwrap_err();
        assert!(matches!(error, LoadError::Hidden(_)), "{error}");
    }

    #[test]
    fn loaded_bill_has_vendor_and_due_date() {
        let Some(books) = load_company_001_or_skip() else {
            return;
        };

        let bill = books
            .bills()
            .iter()
            .find(|bill| bill.id.local() == "bill-operating-00003")
            .expect("bill-operating-00003");
        let vendor = books.vendor(&bill.vendor_id).expect("vendor on the bill");

        assert_eq!(books.company_id().as_str(), "company-001");
        assert_eq!(bill.vendor_id.local(), "vend-landlord");
        assert_eq!(bill.vendor_id.to_string(), "company-001/vend-landlord");
        assert_eq!(bill.vendor_name, "Granite Bay Commercial Properties");
        assert_eq!(vendor.display_name, bill.vendor_name);
        assert_eq!(vendor.supply_role, SupplyRole::Premises);
        assert_eq!(
            bill.due_date,
            NaiveDate::from_ymd_opt(2025, 10, 13).unwrap()
        );
        assert_eq!(bill.total_amount.to_string(), "500.00");
    }

    #[test]
    fn company_001_landlord_rent_is_a_weekly_five_hundred_series() {
        let Some(books) = load_company_001_or_skip() else {
            return;
        };

        // Arrange. SPEC.md §3 calls this "50 weekly $500 landlord bills".
        // The unmodified company-001 extract has 52: vend-landlord, $500.00,
        // due every 7 days from 2025-10-13 through 2026-10-05.
        let landlord = books
            .vendors()
            .iter()
            .find(|vendor| vendor.id.local() == "vend-landlord")
            .expect("landlord vendor");
        let rent: Money = "500.00".parse().unwrap();

        // Act
        let mut bills: Vec<&Bill> = books.bills_for_vendor(&landlord.id).collect();
        let payments: Vec<&BillPayment> = books
            .bill_payments()
            .iter()
            .filter(|payment| payment.vendor_id == landlord.id)
            .collect();

        // Assert
        assert_eq!(landlord.display_name, "Granite Bay Commercial Properties");
        assert_eq!(landlord.supply_role, SupplyRole::Premises);
        assert_eq!(bills.len(), 52, "weekly $500 landlord bills in company-001");
        assert!(bills.iter().all(|bill| bill.total_amount == rent));
        assert!(bills
            .iter()
            .all(|bill| bill.expense_kind == ExpenseKind::Operating));
        assert!(bills
            .iter()
            .all(|bill| books.vendor(&bill.vendor_id).is_some()));
        assert!(bills
            .iter()
            .all(|bill| bill.vendor_name == landlord.display_name));

        bills.sort_by_key(|bill| bill.due_date);
        assert_eq!(
            bills.first().map(|bill| bill.due_date),
            Some(NaiveDate::from_ymd_opt(2025, 10, 13).unwrap())
        );
        assert_eq!(
            bills.last().map(|bill| bill.due_date),
            Some(NaiveDate::from_ymd_opt(2026, 10, 5).unwrap())
        );
        for pair in bills.windows(2) {
            let days = (pair[1].due_date - pair[0].due_date).num_days();
            assert_eq!(days, 7, "landlord due dates are weekly");
        }

        assert_eq!(payments.len(), 52);
        assert!(payments.iter().all(|payment| payment.amount == rent));
    }

    /// Loads the gitignored company-001 extract. Prints a skip line and returns
    /// `None` when `just data` has not been run, so CI without the dataset does
    /// not fail closed on a missing download. A present but unreadable dataset
    /// still fails the test.
    fn load_company_001_or_skip() -> Option<CompanyBooks> {
        let dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/hackathon-100/company-001");
        if !dir.join("rwm-v4").is_dir() {
            eprintln!(
                "SKIP: company-001 dataset not present at {}. Run `just data` in the repo root.",
                dir.display()
            );
            return None;
        }
        match load_company(&dir) {
            Ok(books) => Some(books),
            Err(error) => panic!("company-001 dataset failed to load: {error}"),
        }
    }
}
