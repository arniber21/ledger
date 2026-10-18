//! Per-Specification Filter-width reporting (ADR-0006, ADR-0008).
//!
//! A Pattern declares a per-instance [`FilterWidthBudget`] — the relevance
//! width (event kinds + attribute keys) the Pattern author budgeted for one
//! Specification. The compiler measures the actual width it emitted and reports
//! the delta via [`WidthReport`]. This is the unit-economics / cost-class signal
//! of ADR-0008: a Pattern whose compiled Filter sprawls past its declared
//! budget is a candidate for re-authoring.
//!
//! ## Granularity
//!
//! [`WidthReport`] is keyed by [`FilterIdentity`](crate::FilterIdentity), i.e.
//! per-`SpecificationFilter`. Issue #14 closed the two deferrals this module
//! recorded. The budget is **lifted to the template**: it is declared once on
//! [`WidthBudget`](crate::catalogue::WidthBudget) inside a
//! [`PatternDeclaration`](crate::catalogue::PatternDeclaration), and every
//! instance reports the same one. And the **reports aggregate**, via
//! [`aggregate_by_pattern`], because one Pattern template fans out to many
//! Specifications — SagaShop P4, P6 and P8 are three instances of CAT-1 — and
//! ADR-0008's unit-economics question ("would a single wide Pattern inflate
//! ship volume for every other Specification?") is asked of the template, not
//! of one instance.
//!
//! Aggregation deliberately reports **max** alongside **sum**. Sum is what the
//! deployment pays; max is what the Pattern author is answerable for, since one
//! sprawling instance and ten tidy ones sum alike.
//!
//! ## `over_budget` is telemetry-only
//!
//! `over_budget` is a **cost-class signal**, never a correctness gate. The
//! compiler NEVER truncates an emitted Filter to fit its budget: truncation
//! would drop a relevant event kind or attribute key and could silently invert
//! a Verdict (ADR-0006's central obligation). A future change that turns
//! `over_budget` into a hard error or a truncation trigger is unsound; the
//! `width_never_truncates_test` pins this policy.

#![allow(clippy::module_name_repetitions, clippy::doc_markdown)]

use std::collections::BTreeMap;

use crate::catalogue::PatternId;
use crate::FilterIdentity;

/// Per-Pattern declared Filter-width budget (ADR-0008).
///
/// The Pattern author's commitment that one Specification instance will need at
/// most `event_kinds` distinct Event kinds and `attribute_keys` distinct
/// projected attribute keys. Declared on the Pattern template
/// ([`crate::catalogue::WidthBudget`]) and reported per instance. See the module
/// docs for why the budget is never enforced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilterWidthBudget {
    /// Maximum distinct Event kinds one instance admits.
    pub event_kinds: usize,
    /// Maximum distinct projected attribute keys one instance retains.
    pub attribute_keys: usize,
}

/// Actual-vs-declared width for one compiled [`crate::FilterSource`].
///
/// Built by [`WidthReport::new`] from the deduped union sizes the compiler
/// emitted and the source's declared [`FilterWidthBudget`]. `over_budget` is
/// telemetry-only — see the module docs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WidthReport {
    /// Identity of the Specification this report covers.
    pub identity: FilterIdentity,
    /// Distinct Event kinds actually emitted in the SpecificationFilter.
    pub actual_event_kinds: usize,
    /// Distinct projected attribute keys actually emitted.
    pub actual_attribute_keys: usize,
    /// `actual_event_kinds + actual_attribute_keys`.
    pub actual_width: usize,
    /// Budgeted distinct Event kinds ([`FilterWidthBudget::event_kinds`]).
    pub declared_event_kinds: usize,
    /// Budgeted distinct attribute keys ([`FilterWidthBudget::attribute_keys`]).
    pub declared_attribute_keys: usize,
    /// `declared_event_kinds + declared_attribute_keys`.
    pub declared_width: usize,
    /// `true` iff `actual_width > declared_width`. Telemetry-only; never a gate.
    pub over_budget: bool,
}

impl WidthReport {
    /// Computes a width report from the emitted union sizes and the declared budget.
    ///
    /// `actual_width = actual_kinds + actual_keys`,
    /// `declared_width = budget.event_kinds + budget.attribute_keys`,
    /// `over_budget = actual_width > declared_width`.
    #[must_use]
    pub fn new(
        identity: FilterIdentity,
        actual_kinds: usize,
        actual_keys: usize,
        budget: FilterWidthBudget,
    ) -> Self {
        let actual_width = actual_kinds + actual_keys;
        let declared_width = budget.event_kinds + budget.attribute_keys;
        Self {
            identity,
            actual_event_kinds: actual_kinds,
            actual_attribute_keys: actual_keys,
            actual_width,
            declared_event_kinds: budget.event_kinds,
            declared_attribute_keys: budget.attribute_keys,
            declared_width,
            over_budget: actual_width > declared_width,
        }
    }
}

/// Actual-vs-declared width for every instance of one Pattern template.
///
/// Built by [`aggregate_by_pattern`]. Like [`WidthReport`], every field is
/// telemetry: nothing here gates a compile, and nothing here truncates a Filter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatternWidthReport {
    /// The Pattern template these instances came from.
    pub pattern: PatternId,
    /// How many instances were aggregated.
    pub instances: usize,
    /// Sum of the instances' actual widths — what the deployment pays.
    pub total_actual_width: usize,
    /// Largest single instance's actual width — what the author is answerable for.
    pub max_actual_width: usize,
    /// The template's declared width, identical for every instance.
    ///
    /// `None` when no instance was aggregated, since a declared width with no
    /// instance behind it is the template's constant and not a report.
    pub declared_width: Option<usize>,
    /// How many instances exceeded the declared width.
    pub instances_over_budget: usize,
}

/// Aggregates per-Specification width reports by the Pattern template each came from.
///
/// Takes explicit `(PatternId, WidthReport)` pairs rather than recovering the
/// attribution itself: [`compile_filter`](crate::compile_filter) returns reports
/// in input order and knows nothing about Patterns, so silently zipping the two
/// would make a mis-ordered call a wrong report instead of a compile error. The
/// caller holds both halves and is the only party that can pair them soundly.
///
/// Patterns with no reports are absent from the result rather than present with
/// zeroes — an unused Pattern has no width to report.
#[must_use]
pub fn aggregate_by_pattern(
    reports: &[(PatternId, WidthReport)],
) -> BTreeMap<PatternId, PatternWidthReport> {
    let mut aggregated: BTreeMap<PatternId, PatternWidthReport> = BTreeMap::new();
    for (pattern, report) in reports {
        let entry = aggregated
            .entry(*pattern)
            .or_insert_with(|| PatternWidthReport {
                pattern: *pattern,
                instances: 0,
                total_actual_width: 0,
                max_actual_width: 0,
                declared_width: None,
                instances_over_budget: 0,
            });
        entry.instances += 1;
        entry.total_actual_width += report.actual_width;
        entry.max_actual_width = entry.max_actual_width.max(report.actual_width);
        entry.declared_width = Some(report.declared_width);
        if report.over_budget {
            entry.instances_over_budget += 1;
        }
    }
    aggregated
}
