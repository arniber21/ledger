//! Isolated pin: a non-default [`RawEventRetentionDays`] reaches the sealed
//! envelope via [`Evaluator::with_raw_event_retention`].
//!
//! Kept out of `cold_settlement_watermark` / guard-free digest fixtures so a
//! non-default horizon cannot move those goldens (issue #245 / A6).

mod support;

use sentinel_core::evaluator::Evaluator;
use sentinel_model::RawEventRetentionDays;
use support::{adapted, ship_requires_prior_charge};

#[test]
fn with_raw_event_retention_stamps_non_default_days_on_sealed_envelope() {
    let evidence = adapted("happy");
    let retention = RawEventRetentionDays::try_new(45).expect("45 is in 7..=365");
    let evaluator =
        Evaluator::new(ship_requires_prior_charge()).with_raw_event_retention(retention);
    let sealed = sentinel_core::evaluate(evaluator, evidence.events(), evidence.sources())
        .expect("happy capture evaluates");
    assert!(
        !sealed.is_empty(),
        "fixture must produce at least one Verdict"
    );
    for binding in &sealed {
        assert_eq!(
            binding.verdict().raw_event_retention_days(),
            retention,
            "injected non-default retention must appear on every sealed envelope"
        );
    }
}

#[test]
fn constructor_default_stamps_platform_default_thirty_days() {
    let evidence = adapted("happy");
    let sealed = support::evaluate(&ship_requires_prior_charge(), &evidence).expect("evaluates");
    assert!(!sealed.is_empty());
    for binding in &sealed {
        assert_eq!(
            binding.verdict().raw_event_retention_days(),
            RawEventRetentionDays::platform_default(),
            "digest/default path must keep platform default 30"
        );
    }
}
