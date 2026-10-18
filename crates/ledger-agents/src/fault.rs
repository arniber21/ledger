//! The five faults from SPEC.md section 4, toggled by name.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use crate::error::AgentError;

/// A misbehaving-agent fault. Injected by configuration and run live.
///
/// The dataset files are not edited. Turning a fault on changes the next
/// decision the payer makes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum Fault {
    /// Pay a bill the books already show as settled.
    DoublePay,
    /// Pay a bill that has no approval.
    PayWithoutApproval,
    /// Pay a bill that does not fit the remaining budget.
    ExceedBudget,
    /// Pay a different vendor than the one named on the bill.
    SwappedVendor,
    /// Follow a payment instruction hidden in a bill memo.
    HiddenMemo,
}

impl Fault {
    /// The five faults, in the order their names are documented.
    pub const ALL: [Self; 5] = [
        Self::DoublePay,
        Self::PayWithoutApproval,
        Self::ExceedBudget,
        Self::SwappedVendor,
        Self::HiddenMemo,
    ];

    /// Stable name used on the command line and in span fields.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DoublePay => "double-pay",
            Self::PayWithoutApproval => "no-approval",
            Self::ExceedBudget => "exceed-budget",
            Self::SwappedVendor => "swapped-vendor",
            Self::HiddenMemo => "hidden-memo",
        }
    }

    /// Why a payment that carries this fault needs review.
    #[must_use]
    pub const fn review_reason(self) -> &'static str {
        match self {
            Self::DoublePay => "second payment on a settled obligation",
            Self::PayWithoutApproval => "payment without approval",
            Self::ExceedBudget => "payment exceeds budget",
            Self::SwappedVendor => "payment vendor was swapped",
            Self::HiddenMemo => "agent followed an instruction in a bill memo",
        }
    }

    /// Parses a fault name.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::UnknownFault`] when `name` is not one of
    /// [`Fault::as_str`](Self::as_str).
    pub fn parse(name: &str) -> Result<Self, AgentError> {
        let name = name.trim();
        Self::ALL
            .into_iter()
            .find(|fault| fault.as_str() == name)
            .ok_or_else(|| AgentError::UnknownFault {
                name: name.to_owned(),
            })
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Fault {
    type Err = AgentError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::parse(name)
    }
}

/// Which faults are on for a desk.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FaultConfig {
    enabled: HashSet<Fault>,
}

impl FaultConfig {
    /// No faults. Agents pay open bills inside the budget and ignore memos.
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: HashSet::new(),
        }
    }

    /// Enables each named fault.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::UnknownFault`] when a name is not one of the five.
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<Self, AgentError> {
        let mut config = Self::new();
        for name in names {
            config.enable(Fault::parse(name)?);
        }
        Ok(config)
    }

    /// Turns `fault` on.
    pub fn enable(&mut self, fault: Fault) {
        self.enabled.insert(fault);
    }

    /// Turns `fault` off.
    pub fn disable(&mut self, fault: Fault) {
        self.enabled.remove(&fault);
    }

    /// Flips `fault` and returns whether it is on afterwards.
    #[must_use]
    pub fn toggle(&mut self, fault: Fault) -> bool {
        if self.enabled.contains(&fault) {
            self.enabled.remove(&fault);
            false
        } else {
            self.enabled.insert(fault);
            true
        }
    }

    /// Whether `fault` will change the next decision.
    #[must_use]
    pub fn is_enabled(&self, fault: Fault) -> bool {
        self.enabled.contains(&fault)
    }
}

/// Comma-separated fault names, in [`Fault::ALL`] order.
pub(crate) fn fault_label(faults: &[Fault]) -> Option<String> {
    if faults.is_empty() {
        None
    } else {
        let mut ordered: Vec<Fault> = faults.to_vec();
        ordered.sort_unstable();
        ordered.dedup();
        Some(
            ordered
                .into_iter()
                .map(Fault::as_str)
                .collect::<Vec<_>>()
                .join(","),
        )
    }
}

/// Joined review reasons, in [`Fault::ALL`] order.
pub(crate) fn review_reasons(faults: &[Fault]) -> String {
    let mut ordered: Vec<Fault> = faults.to_vec();
    ordered.sort_unstable();
    ordered.dedup();
    ordered
        .into_iter()
        .map(Fault::review_reason)
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fault_name_round_trips() {
        for fault in Fault::ALL {
            assert_eq!(Fault::parse(fault.as_str()).expect("name"), fault);
            assert!(!fault.review_reason().to_ascii_lowercase().contains("fraud"));
        }
    }

    #[test]
    fn unknown_fault_name_is_rejected() {
        let error = Fault::parse("fraud").expect_err("not a fault");
        assert!(matches!(error, AgentError::UnknownFault { .. }));
    }

    #[test]
    fn toggle_flips_one_fault_and_leaves_the_others() {
        let mut config = FaultConfig::from_names(["double-pay"]).expect("name");
        assert!(config.is_enabled(Fault::DoublePay));
        assert!(!config.toggle(Fault::DoublePay));
        assert!(!config.is_enabled(Fault::DoublePay));
        assert!(config.toggle(Fault::HiddenMemo));
        assert!(config.is_enabled(Fault::HiddenMemo));
        assert!(!config.is_enabled(Fault::ExceedBudget));
    }
}
