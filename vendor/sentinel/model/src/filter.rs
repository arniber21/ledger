//! Validation and canonicalization for the generated Event Filter message.
//!
//! An Event Filter is the versioned subscription a Probe evaluates: ADR-0006 makes it
//! the only channel through which Sentinel asks for evidence, and ADR-0023 fixes the
//! gates a Specification must clear before it may block a caller. [`EventFilter`] is the
//! validated newtype for that message, and `TryFrom<raw_model::EventFilter>` is the only
//! way to construct one.
#![allow(clippy::module_name_repetitions)]

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use prost::Message;

use crate::wire::model::v1 as raw_model;
use crate::wire_helpers::{
    owned_source_key, sort_sources, sort_watermarks, validate_source, validate_watermarks,
    SourceKey, WireError,
};
use crate::{AttributeKey, FilterEpoch, IdentifierError};

/// Validation failure at the Event Filter boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EventFilterError {
    /// Required scalar text is empty.
    MissingScalar(&'static str),
    /// Required message is absent.
    MissingField(&'static str),
    /// Enum number is unknown.
    UnknownEnum {
        /// Enum field name.
        field: &'static str,
        /// Unrecognized numeric value.
        value: i32,
    },
    /// Enum uses its invalid zero sentinel.
    UnspecifiedEnum(&'static str),
    /// A set-like repeated value is duplicated.
    DuplicateValue(&'static str),
    /// Baseline health/completeness evidence was disabled.
    BaselineEvidenceDisabled,
    /// Readiness and evaluation mode form an illegal lifecycle state.
    InvalidReadinessMode,
    /// An enforced Specification did not pass every ADR-0023 gate.
    EnforcementGatesIncomplete,
    /// An enforced Specification is not ask-and-block.
    EnforcementDeliveryMismatch,
    /// An enforced Specification has no latency budget.
    MissingLatencyBudget,
    /// A Specification targets a different Filter epoch than its envelope.
    TargetEpochMismatch,
    /// An active Specification has no authenticated required source set.
    MissingRequiredSources,
    /// An active Specification lacks an activation Watermark for every required source.
    ActivationBarrierIncomplete,
    /// Nested canonical-model evidence is invalid.
    InvalidEvidence(crate::EventError),
    /// An identifier field was empty or whitespace.
    InvalidIdentifier {
        /// Identifier field name.
        field: &'static str,
        /// Identifier validation failure.
        source: IdentifierError,
    },
    /// Protobuf bytes failed to decode.
    Decode(String),
}

impl Display for EventFilterError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingScalar(field) => write!(formatter, "required scalar is missing: {field}"),
            Self::MissingField(field) => write!(formatter, "required message is missing: {field}"),
            Self::UnknownEnum { field, value } => {
                write!(formatter, "unknown {field} enum value: {value}")
            }
            Self::UnspecifiedEnum(field) => write!(formatter, "{field} must not be unspecified"),
            Self::DuplicateValue(field) => {
                write!(formatter, "duplicate value in set-like field: {field}")
            }
            Self::BaselineEvidenceDisabled => formatter
                .write_str("baseline source health and completeness evidence must remain enabled"),
            Self::InvalidReadinessMode => formatter
                .write_str("warming Specifications must remain shadow; active Specifications may shadow, detect, or enforce"),
            Self::EnforcementGatesIncomplete => {
                formatter.write_str("enforcement requires all ADR-0023 gates")
            }
            Self::EnforcementDeliveryMismatch => {
                formatter.write_str("enforcement requires ask-and-block delivery")
            }
            Self::MissingLatencyBudget => {
                formatter.write_str("enforcement requires a nonzero latency budget")
            }
            Self::TargetEpochMismatch => {
                formatter.write_str("Specification target epoch must match Event Filter epoch")
            }
            Self::MissingRequiredSources => {
                formatter.write_str("active Specifications require authenticated source identities")
            }
            Self::ActivationBarrierIncomplete => formatter.write_str(
                "active Specifications require an activation Watermark for every required source",
            ),
            Self::InvalidEvidence(error) => write!(formatter, "invalid model evidence: {error}"),
            Self::InvalidIdentifier { field, source } => {
                write!(formatter, "invalid identifier in {field}: {source}")
            }
            Self::Decode(error) => write!(formatter, "invalid protobuf bytes: {error}"),
        }
    }
}

impl Error for EventFilterError {
    // Every variant is listed: no `_` arm, so adding or renaming one is a compile error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidEvidence(error) => Some(error),
            Self::InvalidIdentifier { source, .. } => Some(source),
            Self::MissingScalar(_)
            | Self::MissingField(_)
            | Self::UnknownEnum { .. }
            | Self::UnspecifiedEnum(_)
            | Self::DuplicateValue(_)
            | Self::BaselineEvidenceDisabled
            | Self::InvalidReadinessMode
            | Self::EnforcementGatesIncomplete
            | Self::EnforcementDeliveryMismatch
            | Self::MissingLatencyBudget
            | Self::TargetEpochMismatch
            | Self::MissingRequiredSources
            | Self::ActivationBarrierIncomplete
            | Self::Decode(_) => None,
        }
    }
}

impl From<WireError> for EventFilterError {
    fn from(error: WireError) -> Self {
        match error {
            WireError::MissingField(field) => Self::MissingField(field),
            WireError::DuplicateValue(field) => Self::DuplicateValue(field),
            WireError::InvalidEvidence(error) => Self::InvalidEvidence(error),
        }
    }
}

fn require_text(value: &str, field: &'static str) -> Result<(), EventFilterError> {
    if value.trim().is_empty() {
        Err(EventFilterError::MissingScalar(field))
    } else {
        Ok(())
    }
}

fn parse_fail_mode(value: i32) -> Result<raw_model::FailMode, EventFilterError> {
    match raw_model::FailMode::try_from(value).map_err(|_| EventFilterError::UnknownEnum {
        field: "fail_mode",
        value,
    })? {
        raw_model::FailMode::Unspecified => Err(EventFilterError::UnspecifiedEnum("fail_mode")),
        mode => Ok(mode),
    }
}

fn canonicalize_filter(filter: &mut raw_model::EventFilter) {
    for specification in &mut filter.specifications {
        if let Some(event_match) = &mut specification.event_match {
            event_match.event_kinds.sort();
            event_match.projected_attribute_keys.sort();
        }
        sort_sources(&mut specification.required_sources);
        sort_watermarks(&mut specification.activation_watermarks);
    }
    filter.specifications.sort_by(|left, right| {
        (&left.specification_id, &left.specification_version)
            .cmp(&(&right.specification_id, &right.specification_version))
    });
}

/// Validated Event Filter backed only by generated storage.
#[derive(Clone, Debug, PartialEq)]
pub struct EventFilter(raw_model::EventFilter);

impl EventFilter {
    /// Encodes the canonical Filter in Protobuf wire format.
    #[must_use]
    pub fn encode_to_vec(&self) -> Vec<u8> {
        self.0.encode_to_vec()
    }

    /// Decodes and validates an untrusted wire message.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed bytes or a decoded message that
    /// violates Event Filter invariants.
    pub fn decode(bytes: &[u8]) -> Result<Self, EventFilterError> {
        let raw = raw_model::EventFilter::decode(bytes)
            .map_err(|error| EventFilterError::Decode(error.to_string()))?;
        Self::try_from(raw)
    }

    /// Returns the effective blocking budget for matched active-enforce Specifications.
    ///
    /// The caller supplies the request's relative remaining transport budget. The result
    /// is its minimum with every matched per-Specification budget. Local monotonic clocks
    /// decrement the request value between hops; no absolute clock is compared.
    #[must_use]
    pub fn effective_budget_nanoseconds(
        &self,
        request_remaining: u64,
        matched_specification_ids: &BTreeSet<&str>,
    ) -> u64 {
        self.0
            .specifications
            .iter()
            .filter(|specification| {
                matched_specification_ids.contains(specification.specification_id.as_str())
                    && specification.evaluation_mode == raw_model::EvaluationMode::Enforce as i32
                    && specification.readiness == raw_model::Readiness::Active as i32
            })
            .filter_map(|specification| specification.latency_budget_nanoseconds)
            .fold(request_remaining, u64::min)
    }
    /// Returns the envelope's Filter epoch.
    ///
    /// Already validated to be the epoch every entry targets, so this is the
    /// value a served decision must be computed against: the Sentinel-side
    /// seam (issue #22) cross-checks it against the request's acknowledged
    /// epoch before answering.
    ///
    /// # Panics
    ///
    /// Never: the epoch is validated at construction.
    #[must_use]
    pub fn epoch(&self) -> FilterEpoch {
        FilterEpoch::new(self.0.epoch.expect("validated Filter epoch"))
    }

    /// Returns the validated, canonically ordered Specification entries.
    ///
    /// Read-only by construction: the unresolved-decision aggregation
    /// ([`crate::aggregate_unresolved`]) looks entries up by identity
    /// in-crate, and the Sentinel-side decision seam (issue #22) recomputes
    /// the matched active-enforcement set from the same validated payload
    /// rather than from a projected copy that could drift from the wire — the
    /// second implementation ADR-0027 refuses.
    #[must_use]
    pub fn specifications(&self) -> &[raw_model::SpecificationFilter] {
        &self.0.specifications
    }
}

fn validate_event_match(
    specification: &raw_model::SpecificationFilter,
) -> Result<raw_model::DeliveryMode, EventFilterError> {
    let event_match = specification
        .event_match
        .as_ref()
        .ok_or(EventFilterError::MissingField("specification.event_match"))?;
    let delivery = raw_model::DeliveryMode::try_from(event_match.delivery_mode).map_err(|_| {
        EventFilterError::UnknownEnum {
            field: "delivery_mode",
            value: event_match.delivery_mode,
        }
    })?;
    if delivery == raw_model::DeliveryMode::Unspecified {
        return Err(EventFilterError::UnspecifiedEnum("delivery_mode"));
    }
    if event_match.event_kinds.is_empty() {
        return Err(EventFilterError::MissingField("event_match.event_kinds"));
    }
    let mut event_kinds = BTreeSet::new();
    for event_kind in &event_match.event_kinds {
        require_text(event_kind, "event_match.event_kinds")?;
        if !event_kinds.insert(event_kind) {
            return Err(EventFilterError::DuplicateValue("event_match.event_kinds"));
        }
    }
    let mut projected_keys = BTreeSet::new();
    for key in &event_match.projected_attribute_keys {
        AttributeKey::new(key.clone()).map_err(|source| EventFilterError::InvalidIdentifier {
            field: "event_match.projected_attribute_keys",
            source,
        })?;
        if !projected_keys.insert(key) {
            return Err(EventFilterError::DuplicateValue(
                "event_match.projected_attribute_keys",
            ));
        }
    }
    Ok(delivery)
}

fn validate_specification_sources(
    specification: &raw_model::SpecificationFilter,
) -> Result<(BTreeSet<SourceKey>, BTreeSet<SourceKey>), EventFilterError> {
    let mut required_sources = BTreeSet::new();
    for source in &specification.required_sources {
        validate_source(source)?;
        if !required_sources.insert(owned_source_key(source)) {
            return Err(EventFilterError::DuplicateValue("required_sources"));
        }
    }
    let activated_sources = validate_watermarks(
        &specification.activation_watermarks,
        "activation_watermarks",
    )?;
    Ok((required_sources, activated_sources))
}

fn validate_activation_barrier(
    required_sources: &BTreeSet<SourceKey>,
    activated_sources: &BTreeSet<SourceKey>,
) -> Result<(), EventFilterError> {
    if required_sources.is_empty() {
        return Err(EventFilterError::MissingRequiredSources);
    }
    if activated_sources == required_sources {
        Ok(())
    } else {
        Err(EventFilterError::ActivationBarrierIncomplete)
    }
}

fn validate_enforcement(
    specification: &raw_model::SpecificationFilter,
    delivery: raw_model::DeliveryMode,
    required_sources: &BTreeSet<SourceKey>,
    activated_sources: &BTreeSet<SourceKey>,
) -> Result<(), EventFilterError> {
    if delivery != raw_model::DeliveryMode::AskAndBlock {
        return Err(EventFilterError::EnforcementDeliveryMismatch);
    }
    if !matches!(specification.latency_budget_nanoseconds, Some(1..)) {
        return Err(EventFilterError::MissingLatencyBudget);
    }
    validate_activation_barrier(required_sources, activated_sources)?;
    let gates = specification
        .enforcement_gates
        .as_ref()
        .ok_or(EventFilterError::EnforcementGatesIncomplete)?;
    if gates.safety_property
        && gates.ask_and_block
        && gates.hot_path_only
        && gates.anchor_tier
        && gates.shadow_and_adversarial_validation_passed
    {
        Ok(())
    } else {
        Err(EventFilterError::EnforcementGatesIncomplete)
    }
}

impl TryFrom<raw_model::EventFilter> for EventFilter {
    type Error = EventFilterError;

    fn try_from(mut value: raw_model::EventFilter) -> Result<Self, Self::Error> {
        let filter_epoch = value
            .epoch
            .ok_or(EventFilterError::MissingScalar("event_filter.epoch"))?;
        let baseline = value
            .baseline_evidence
            .as_ref()
            .ok_or(EventFilterError::MissingField(
                "event_filter.baseline_evidence",
            ))?;
        if !baseline.source_health || !baseline.completeness_signals {
            return Err(EventFilterError::BaselineEvidenceDisabled);
        }
        let mut identities = BTreeSet::new();
        for specification in &value.specifications {
            require_text(&specification.specification_id, "specification_id")?;
            require_text(
                &specification.specification_version,
                "specification_version",
            )?;
            if !identities.insert((
                &specification.specification_id,
                &specification.specification_version,
            )) {
                return Err(EventFilterError::DuplicateValue("specification identity"));
            }
            let delivery = validate_event_match(specification)?;
            let evaluation = raw_model::EvaluationMode::try_from(specification.evaluation_mode)
                .map_err(|_| EventFilterError::UnknownEnum {
                    field: "evaluation_mode",
                    value: specification.evaluation_mode,
                })?;
            if evaluation == raw_model::EvaluationMode::Unspecified {
                return Err(EventFilterError::UnspecifiedEnum("evaluation_mode"));
            }
            let readiness =
                raw_model::Readiness::try_from(specification.readiness).map_err(|_| {
                    EventFilterError::UnknownEnum {
                        field: "readiness",
                        value: specification.readiness,
                    }
                })?;
            if readiness == raw_model::Readiness::Unspecified {
                return Err(EventFilterError::UnspecifiedEnum("readiness"));
            }
            if readiness == raw_model::Readiness::Warming
                && evaluation != raw_model::EvaluationMode::Shadow
            {
                return Err(EventFilterError::InvalidReadinessMode);
            }
            parse_fail_mode(specification.fail_mode)?;
            if specification.target_filter_epoch != Some(filter_epoch) {
                return Err(EventFilterError::TargetEpochMismatch);
            }
            let (required_sources, activated_sources) =
                validate_specification_sources(specification)?;
            let barrier_required =
                readiness == raw_model::Readiness::Active || !activated_sources.is_empty();
            if barrier_required {
                validate_activation_barrier(&required_sources, &activated_sources)?;
            }
            if evaluation == raw_model::EvaluationMode::Enforce {
                validate_enforcement(
                    specification,
                    delivery,
                    &required_sources,
                    &activated_sources,
                )?;
            }
        }
        canonicalize_filter(&mut value);
        Ok(Self(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `EventFilterError` variant paired with the exact `Display` output it must produce.
    ///
    /// Carried over verbatim from the single-error table this type was split out of (#65).
    fn error_display_cases() -> Vec<(EventFilterError, &'static str)> {
        vec![
            (
                EventFilterError::MissingScalar("field"),
                "required scalar is missing: field",
            ),
            (
                EventFilterError::MissingField("field"),
                "required message is missing: field",
            ),
            (
                EventFilterError::UnknownEnum {
                    field: "field",
                    value: 7,
                },
                "unknown field enum value: 7",
            ),
            (
                EventFilterError::UnspecifiedEnum("field"),
                "field must not be unspecified",
            ),
            (
                EventFilterError::DuplicateValue("field"),
                "duplicate value in set-like field: field",
            ),
            (
                EventFilterError::BaselineEvidenceDisabled,
                "baseline source health and completeness evidence must remain enabled",
            ),
            (
                EventFilterError::InvalidReadinessMode,
                "warming Specifications must remain shadow; active Specifications may shadow, detect, or enforce",
            ),
            (
                EventFilterError::EnforcementGatesIncomplete,
                "enforcement requires all ADR-0023 gates",
            ),
            (
                EventFilterError::EnforcementDeliveryMismatch,
                "enforcement requires ask-and-block delivery",
            ),
            (
                EventFilterError::MissingLatencyBudget,
                "enforcement requires a nonzero latency budget",
            ),
            (
                EventFilterError::TargetEpochMismatch,
                "Specification target epoch must match Event Filter epoch",
            ),
            (
                EventFilterError::MissingRequiredSources,
                "active Specifications require authenticated source identities",
            ),
            (
                EventFilterError::ActivationBarrierIncomplete,
                "active Specifications require an activation Watermark for every required source",
            ),
            (
                EventFilterError::InvalidEvidence(crate::EventError::SelfCausalPredecessor),
                "invalid model evidence: event must not be its own causal predecessor",
            ),
            (
                EventFilterError::InvalidIdentifier {
                    field: "field",
                    source: IdentifierError,
                },
                "invalid identifier in field: identifier must not be empty or whitespace",
            ),
            (
                EventFilterError::Decode("field".to_owned()),
                "invalid protobuf bytes: field",
            ),
        ]
    }

    #[test]
    fn error_display_is_stable() {
        let cases = error_display_cases();
        assert_eq!(
            cases.len(),
            16,
            "every EventFilterError variant must be pinned"
        );
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected, "{error:?}");
        }
    }

    #[test]
    fn error_source_is_exposed_only_for_wrapped_errors() {
        for (error, _) in error_display_cases() {
            let exposes_source = matches!(
                error,
                EventFilterError::InvalidEvidence(_) | EventFilterError::InvalidIdentifier { .. }
            );
            assert_eq!(error.source().is_some(), exposes_source, "{error:?}");
        }
    }

    fn baseline() -> raw_model::BaselineEvidenceFilter {
        raw_model::BaselineEvidenceFilter {
            source_health: true,
            completeness_signals: true,
        }
    }

    fn source() -> raw_model::SourceIdentity {
        raw_model::SourceIdentity {
            monitored_application_id: "app".to_owned(),
            source_id: "gateway".to_owned(),
        }
    }

    fn observed_at() -> raw_model::ObservationTime {
        raw_model::ObservationTime {
            sentinel_clock_id: "sentinel".to_owned(),
            nanoseconds: Some(raw_model::Int128::default()),
        }
    }

    fn watermark() -> raw_model::Watermark {
        raw_model::Watermark {
            source: Some(source()),
            sequence_epoch: 1,
            complete_through: 4,
            observed_at: Some(observed_at()),
        }
    }

    fn second_source() -> raw_model::SourceIdentity {
        raw_model::SourceIdentity {
            monitored_application_id: "app".to_owned(),
            source_id: "authz".to_owned(),
        }
    }

    fn shadow_filter(
        readiness: raw_model::Readiness,
        required_sources: Vec<raw_model::SourceIdentity>,
        activation_watermarks: Vec<raw_model::Watermark>,
    ) -> raw_model::EventFilter {
        raw_model::EventFilter {
            epoch: Some(9),
            specifications: vec![raw_model::SpecificationFilter {
                specification_id: "spec".to_owned(),
                specification_version: "v1".to_owned(),
                event_match: Some(raw_model::EventMatch {
                    event_kinds: vec!["tool.requested".to_owned()],
                    projected_attribute_keys: vec!["amount".to_owned()],
                    delivery_mode: raw_model::DeliveryMode::ShipAsync as i32,
                }),
                fail_mode: raw_model::FailMode::Open as i32,
                latency_budget_nanoseconds: None,
                evaluation_mode: raw_model::EvaluationMode::Shadow as i32,
                readiness: readiness as i32,
                target_filter_epoch: Some(9),
                required_sources,
                activation_watermarks,
                enforcement_gates: None,
            }],
            baseline_evidence: Some(baseline()),
        }
    }

    fn active_filter(activation_watermarks: Vec<raw_model::Watermark>) -> raw_model::EventFilter {
        raw_model::EventFilter {
            epoch: Some(9),
            specifications: vec![raw_model::SpecificationFilter {
                specification_id: "spec".to_owned(),
                specification_version: "v1".to_owned(),
                event_match: Some(raw_model::EventMatch {
                    event_kinds: vec!["tool.requested".to_owned()],
                    projected_attribute_keys: vec!["amount".to_owned()],
                    delivery_mode: raw_model::DeliveryMode::AskAndBlock as i32,
                }),
                fail_mode: raw_model::FailMode::Open as i32,
                latency_budget_nanoseconds: Some(50),
                evaluation_mode: raw_model::EvaluationMode::Enforce as i32,
                readiness: raw_model::Readiness::Active as i32,
                target_filter_epoch: Some(9),
                required_sources: vec![source()],
                activation_watermarks,
                enforcement_gates: Some(raw_model::EnforcementGateEvidence {
                    safety_property: true,
                    ask_and_block: true,
                    hot_path_only: true,
                    anchor_tier: true,
                    shadow_and_adversarial_validation_passed: true,
                }),
            }],
            baseline_evidence: Some(baseline()),
        }
    }

    #[test]
    fn readiness_and_evaluation_mode_matrix_is_enforced() {
        for readiness in [raw_model::Readiness::Warming, raw_model::Readiness::Active] {
            for evaluation in [
                raw_model::EvaluationMode::Shadow,
                raw_model::EvaluationMode::Detect,
                raw_model::EvaluationMode::Enforce,
            ] {
                let mut filter = active_filter(vec![watermark()]);
                filter.specifications[0].readiness = readiness as i32;
                filter.specifications[0].evaluation_mode = evaluation as i32;
                let result = EventFilter::try_from(filter);
                if readiness == raw_model::Readiness::Warming
                    && evaluation != raw_model::EvaluationMode::Shadow
                {
                    assert_eq!(result, Err(EventFilterError::InvalidReadinessMode));
                } else {
                    result.expect("legal readiness/evaluation combination");
                }
            }
        }
    }

    #[test]
    fn active_shadow_with_empty_watermarks_is_activation_barrier_incomplete() {
        assert_eq!(
            EventFilter::try_from(shadow_filter(
                raw_model::Readiness::Active,
                vec![source()],
                Vec::new(),
            )),
            Err(EventFilterError::ActivationBarrierIncomplete)
        );
    }

    #[test]
    fn active_detect_with_empty_watermarks_is_activation_barrier_incomplete() {
        let mut filter = shadow_filter(raw_model::Readiness::Active, vec![source()], Vec::new());
        filter.specifications[0].evaluation_mode = raw_model::EvaluationMode::Detect as i32;
        assert_eq!(
            EventFilter::try_from(filter),
            Err(EventFilterError::ActivationBarrierIncomplete)
        );
    }

    #[test]
    fn warming_shadow_with_empty_watermarks_is_valid() {
        EventFilter::try_from(shadow_filter(
            raw_model::Readiness::Warming,
            vec![source()],
            Vec::new(),
        ))
        .expect("Warming + Shadow + empty watermarks remains the unpublished compile path");
    }

    #[test]
    fn warming_shadow_with_partial_watermarks_is_activation_barrier_incomplete() {
        assert_eq!(
            EventFilter::try_from(shadow_filter(
                raw_model::Readiness::Warming,
                vec![source(), second_source()],
                vec![watermark()],
            )),
            Err(EventFilterError::ActivationBarrierIncomplete)
        );
    }

    #[test]
    fn warming_shadow_with_complete_watermarks_is_valid() {
        EventFilter::try_from(shadow_filter(
            raw_model::Readiness::Warming,
            vec![source()],
            vec![watermark()],
        ))
        .expect("Warming + Shadow + a complete barrier is a published Filter");
    }

    #[test]
    fn effective_budget_is_minimum_of_request_and_active_enforcement() {
        let filter =
            EventFilter::try_from(active_filter(vec![watermark()])).expect("valid active filter");

        assert_eq!(
            filter.effective_budget_nanoseconds(100, &BTreeSet::from(["spec"])),
            50
        );
    }

    #[test]
    fn active_enforcement_rejects_malformed_observation_time() {
        let malformed = raw_model::Watermark {
            observed_at: Some(raw_model::ObservationTime::default()),
            ..watermark()
        };
        assert_eq!(
            EventFilter::try_from(active_filter(vec![malformed])),
            Err(EventFilterError::InvalidEvidence(
                crate::EventError::InvalidIdentifier {
                    field: "observation_time.sentinel_clock_id",
                    source: crate::IdentifierError,
                }
            ))
        );
    }

    #[test]
    fn active_enforcement_rejects_duplicate_activation_watermark_sources() {
        assert_eq!(
            EventFilter::try_from(active_filter(vec![watermark(), watermark()])),
            Err(EventFilterError::DuplicateValue("activation_watermarks"))
        );
    }

    #[test]
    fn decode_rejects_garbage_bytes() {
        assert!(matches!(
            EventFilter::decode(&[0xff, 0x00, 0x01]),
            Err(EventFilterError::Decode(_))
        ));
    }

    #[test]
    fn decode_round_trips_through_encode_to_vec() {
        let filter =
            EventFilter::try_from(active_filter(vec![watermark()])).expect("valid active Filter");
        let decoded = EventFilter::decode(&filter.encode_to_vec()).expect("round-trip decodes");
        assert_eq!(decoded, filter);
    }

    #[test]
    fn epoch_accessor_returns_the_validated_envelope_epoch() {
        let filter =
            EventFilter::try_from(active_filter(vec![watermark()])).expect("valid active Filter");
        assert_eq!(filter.epoch(), FilterEpoch::new(9));

        let mut stale = active_filter(vec![watermark()]);
        stale.epoch = Some(8);
        assert_eq!(
            EventFilter::try_from(stale),
            Err(EventFilterError::TargetEpochMismatch),
            "the accessor only ever exposes an epoch the entries target"
        );
    }

    #[test]
    fn blank_projected_attribute_keys_are_invalid_identifiers() {
        let mut filter = active_filter(vec![watermark()]);
        filter.specifications[0]
            .event_match
            .as_mut()
            .expect("fixture event match")
            .projected_attribute_keys = vec!["   ".to_owned()];
        assert_eq!(
            EventFilter::try_from(filter),
            Err(EventFilterError::InvalidIdentifier {
                field: "event_match.projected_attribute_keys",
                source: IdentifierError,
            })
        );
    }
}
