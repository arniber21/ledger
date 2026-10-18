//! Decision-table pins for CAT-3 existence.before-scope-close (#210).

use sentinel_core::existence::{
    aggregate, decide, AnchorSilence, ConcurrentUncertainty, CoverageState, ExistenceObservation,
    ExistenceOutcome, GateState, RequiredPresence, ScopeClose,
};

#[test]
fn satisfy_at_close_with_required_in_interval() {
    let observation = ExistenceObservation::new(
        GateState::Open,
        ScopeClose::Present,
        RequiredPresence::Present,
        ConcurrentUncertainty::Absent,
        AnchorSilence::Available,
        CoverageState::Settled,
    );
    assert_eq!(decide(observation), ExistenceOutcome::Satisfy);
}

#[test]
fn violate_at_close_without_required_when_anchor_silence_available() {
    let observation = ExistenceObservation::new(
        GateState::Open,
        ScopeClose::Present,
        RequiredPresence::Absent,
        ConcurrentUncertainty::Absent,
        AnchorSilence::Available,
        CoverageState::Settled,
    );
    assert_eq!(decide(observation), ExistenceOutcome::Violate);
}

#[test]
fn contributing_only_silence_does_not_convict() {
    let observation = ExistenceObservation::new(
        GateState::Open,
        ScopeClose::Present,
        RequiredPresence::Absent,
        ConcurrentUncertainty::Absent,
        AnchorSilence::Unavailable,
        CoverageState::Settled,
    );
    assert_eq!(decide(observation), ExistenceOutcome::InconclusivePrefix);
}

#[test]
fn open_scope_without_close_is_prefix_never_satisfy() {
    let observation = ExistenceObservation::new(
        GateState::Open,
        ScopeClose::Absent,
        RequiredPresence::Present,
        ConcurrentUncertainty::Absent,
        AnchorSilence::Available,
        CoverageState::Settled,
    );
    assert_eq!(decide(observation), ExistenceOutcome::InconclusivePrefix);
}

#[test]
fn concurrent_open_required_close_is_observational() {
    let observation = ExistenceObservation::new(
        GateState::Open,
        ScopeClose::Present,
        RequiredPresence::Present,
        ConcurrentUncertainty::Present,
        AnchorSilence::Available,
        CoverageState::Settled,
    );
    assert_eq!(
        decide(observation),
        ExistenceOutcome::InconclusiveObservational
    );
}

#[test]
fn binding_dominance_is_universal_conjunction() {
    assert_eq!(
        aggregate([
            ExistenceOutcome::Satisfy,
            ExistenceOutcome::InconclusiveObservational,
            ExistenceOutcome::Violate,
        ]),
        Some(ExistenceOutcome::Violate)
    );
    assert_eq!(
        aggregate([
            ExistenceOutcome::Satisfy,
            ExistenceOutcome::InconclusivePrefix
        ]),
        Some(ExistenceOutcome::InconclusivePrefix)
    );
    assert_eq!(aggregate(std::iter::empty::<ExistenceOutcome>()), None);
}

#[test]
fn closed_gate_wins_outright() {
    let observation = ExistenceObservation::new(
        GateState::Closed,
        ScopeClose::Present,
        RequiredPresence::Present,
        ConcurrentUncertainty::Absent,
        AnchorSilence::Available,
        CoverageState::Settled,
    );
    assert_eq!(decide(observation), ExistenceOutcome::InconclusivePrefix);
}
