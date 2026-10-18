//! Company-scoped identifiers.
//!
//! Dataset ids such as `vend-landlord` are unique only inside one company.
//! Two companies can both have a `bill-00005`. [`CompanyId`] is the scope;
//! each entity id pairs that scope with the local id from the YAML.

use std::fmt;
use std::sync::Arc;

use thiserror::Error;

/// Directory name of a loaded company, for example `company-001`.
///
/// This is the scope for every id in that company's books. It is the folder
/// name under `data/hackathon-100/`, not the in-file profile id `company-main`,
/// which repeats across companies.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CompanyId(Arc<str>);

impl CompanyId {
    /// Parses a company scope.
    ///
    /// # Errors
    ///
    /// Returns [`IdError::Invalid`] when `id` is empty, `hidden`, `.`, `..`,
    /// or contains whitespace or a path separator.
    pub fn new(id: &str) -> Result<Self, IdError> {
        if !is_safe_id(id) {
            return Err(IdError::Invalid);
        }
        Ok(Self(Arc::from(id)))
    }

    /// The company scope as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CompanyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An identifier that is only meaningful inside one [`CompanyId`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScopedId<Kind> {
    company: CompanyId,
    local: Arc<str>,
    kind: std::marker::PhantomData<Kind>,
}

impl<Kind> ScopedId<Kind> {
    /// Pairs `company` with a local id from the dataset.
    ///
    /// # Errors
    ///
    /// Returns [`IdError::Invalid`] when `local` is empty, `hidden`, `.`, `..`,
    /// or contains whitespace or a path separator.
    pub fn new(company: CompanyId, local: &str) -> Result<Self, IdError> {
        if !is_safe_id(local) {
            return Err(IdError::Invalid);
        }
        Ok(Self {
            company,
            local: Arc::from(local),
            kind: std::marker::PhantomData,
        })
    }

    /// Company this id belongs to.
    #[must_use]
    pub fn company(&self) -> &CompanyId {
        &self.company
    }

    /// Id as it appears in the company's YAML.
    #[must_use]
    pub fn local(&self) -> &str {
        &self.local
    }
}

impl<Kind> fmt::Display for ScopedId<Kind> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.company, self.local)
    }
}

/// Tag for a vendor id. Not constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VendorKind {
    _private: (),
}

/// Tag for a bill id. Not constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BillKind {
    _private: (),
}

/// Tag for a bill-payment id. Not constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BillPaymentKind {
    _private: (),
}

/// Tag for an item id. Not constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemKind {
    _private: (),
}

/// Tag for an account id. Not constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccountKind {
    _private: (),
}

/// Tag for a project id. Not constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProjectKind {
    _private: (),
}

/// Vendor id scoped to one company.
pub type VendorId = ScopedId<VendorKind>;

/// Bill id scoped to one company.
pub type BillId = ScopedId<BillKind>;

/// Bill-payment id scoped to one company.
pub type BillPaymentId = ScopedId<BillPaymentKind>;

/// Item id scoped to one company.
pub type ItemId = ScopedId<ItemKind>;

/// Account id scoped to one company.
pub type AccountId = ScopedId<AccountKind>;

/// Project id scoped to one company.
pub type ProjectId = ScopedId<ProjectKind>;

/// A company scope or local id was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum IdError {
    /// The string was empty, `hidden`, a path dot, or contained whitespace or a separator.
    #[error("invalid company-scoped identifier")]
    Invalid,
}

fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id != "hidden"
        && id != "."
        && id != ".."
        && !id.contains('/')
        && !id.contains('\\')
        && !id.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_local_id_in_two_companies_is_not_equal() {
        let left = VendorId::new(CompanyId::new("company-001").unwrap(), "vend-landlord").unwrap();
        let right = VendorId::new(CompanyId::new("company-002").unwrap(), "vend-landlord").unwrap();

        assert_ne!(left, right);
        assert_eq!(left.to_string(), "company-001/vend-landlord");
    }

    #[test]
    fn rejects_hidden_and_empty_ids() {
        assert!(CompanyId::new("hidden").is_err());
        assert!(CompanyId::new("").is_err());
        assert!(CompanyId::new("company-001/hidden").is_err());
        let company = CompanyId::new("company-001").unwrap();
        assert!(VendorId::new(company, "hidden").is_err());
    }
}
