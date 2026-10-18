//! Validation and deterministic aggregation for the generated Probe decision messages.
//!
//! ADR-0023 fixes what a Probe may do when Sentinel cannot answer in time: the request
//! carries a relative transport budget and producer evidence bound to a Filter epoch, and
//! an unresolved response must account for every matched active-enforcement Specification
//! with the Filter's own declared fail mode. [`DecideRequest`] is the validated newtype
//! for the request; [`aggregate_unresolved`] is the deterministic fail-mode fold over the
//! response.
#![allow(clippy::missing_panics_doc)]

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use prost::Message;

use crate::filter::EventFilter;
use crate::wire::{model::v1 as raw_model, probe::v1 as raw_probe};
use crate::{FilterEpoch, ProducerEvent};

/// Validation failure at the Probe decision boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DecideRequestError {
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
    /// The request carries no nonzero transport budget.
    MissingTransportBudget,
    /// Request and producer evidence acknowledge different Filter epochs.
    FilterEpochMismatch,
    /// An unresolved decision omitted a named reason.
    MissingUnresolvedReason,
    /// An unresolved response contained no per-Specification decisions.
    MissingUnresolvedDecisions,
    /// A decision response or record was not DEFER at the unresolved boundary.
    UnresolvedActionMismatch,
    /// A decision did not belong to the matched active-enforcement set.
    IrrelevantSpecificationDecision,
    /// A decision's fail mode differed from its active Filter entry.
    DecisionFailModeMismatch,
    /// The unresolved response omitted a matched active-enforcement Specification.
    IncompleteSpecificationDecisions,
    /// Embedded producer evidence is invalid.
    InvalidProducer(crate::EventError),
}

impl Display for DecideRequestError {
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
            Self::MissingTransportBudget => {
                formatter.write_str("request requires a nonzero transport budget")
            }
            Self::FilterEpochMismatch => {
                formatter.write_str("request and producer Filter epochs differ")
            }
            Self::MissingUnresolvedReason => {
                formatter.write_str("deferred decision requires an unresolved reason")
            }
            Self::MissingUnresolvedDecisions => formatter
                .write_str("deferred response requires at least one unresolved Specification"),
            Self::UnresolvedActionMismatch => formatter
                .write_str("unresolved response and every Specification decision must defer"),
            Self::IrrelevantSpecificationDecision => formatter
                .write_str("unresolved decision is not a matched active-enforcement Specification"),
            Self::DecisionFailModeMismatch => {
                formatter.write_str("unresolved decision fail mode differs from the active Filter")
            }
            Self::IncompleteSpecificationDecisions => formatter.write_str(
                "unresolved response does not cover every matched active-enforcement Specification",
            ),
            Self::InvalidProducer(error) => write!(formatter, "invalid producer event: {error}"),
        }
    }
}

impl Error for DecideRequestError {
    // Every variant is listed: no `_` arm, so adding or renaming one is a compile error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidProducer(error) => Some(error),
            Self::MissingScalar(_)
            | Self::MissingField(_)
            | Self::UnknownEnum { .. }
            | Self::UnspecifiedEnum(_)
            | Self::DuplicateValue(_)
            | Self::MissingTransportBudget
            | Self::FilterEpochMismatch
            | Self::MissingUnresolvedReason
            | Self::MissingUnresolvedDecisions
            | Self::UnresolvedActionMismatch
            | Self::IrrelevantSpecificationDecision
            | Self::DecisionFailModeMismatch
            | Self::IncompleteSpecificationDecisions => None,
        }
    }
}

fn require_text(value: &str, field: &'static str) -> Result<(), DecideRequestError> {
    if value.trim().is_empty() {
        Err(DecideRequestError::MissingScalar(field))
    } else {
        Ok(())
    }
}

fn parse_fail_mode(value: i32) -> Result<raw_model::FailMode, DecideRequestError> {
    match raw_model::FailMode::try_from(value).map_err(|_| DecideRequestError::UnknownEnum {
        field: "fail_mode",
        value,
    })? {
        raw_model::FailMode::Unspecified => Err(DecideRequestError::UnspecifiedEnum("fail_mode")),
        mode => Ok(mode),
    }
}

fn parse_action(value: i32) -> Result<raw_probe::DecisionAction, DecideRequestError> {
    match raw_probe::DecisionAction::try_from(value).map_err(|_| {
        DecideRequestError::UnknownEnum {
            field: "decision_action",
            value,
        }
    })? {
        raw_probe::DecisionAction::Unspecified => {
            Err(DecideRequestError::UnspecifiedEnum("decision_action"))
        }
        action => Ok(action),
    }
}

/// Validated Probe decision request backed by generated storage.
#[derive(Clone, Debug, PartialEq)]
pub struct DecideRequest(raw_probe::DecideRequest);

impl DecideRequest {
    /// Encodes the canonical request in Protobuf wire format.
    #[must_use]
    pub fn encode_to_vec(&self) -> Vec<u8> {
        self.0.encode_to_vec()
    }

    /// Returns the request's identifier, echoed verbatim into the response.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.0.request_id
    }

    /// Returns the Filter epoch the request acknowledges.
    ///
    /// Validated at construction to equal the producer evidence's own
    /// acknowledged epoch, so this is the Probe's word about which Filter
    /// epoch it evaluated against — the value the Sentinel-side seam (issue
    /// #22) cross-checks against the handed Filter's own epoch.
    #[must_use]
    pub fn filter_epoch(&self) -> FilterEpoch {
        FilterEpoch::new(self.0.filter_epoch.expect("validated Filter epoch"))
    }

    /// Returns the relative transport budget remaining at this hop.
    #[must_use]
    pub fn remaining_transport_budget_nanoseconds(&self) -> u64 {
        self.0
            .remaining_transport_budget_nanoseconds
            .expect("validated transport budget")
    }

    /// Returns the producer evidence the request asked about.
    ///
    /// Validated at construction: identity, kind, attributes and declared
    /// predecessors are the asked action. `ObservationTime`, Data Class and
    /// `SourceAssurance` are not on this type and cannot be invented here.
    #[must_use]
    pub fn producer(&self) -> ProducerEvent {
        ProducerEvent(self.0.producer_event.clone().expect("validated producer"))
    }
}
impl TryFrom<raw_probe::DecideRequest> for DecideRequest {
    type Error = DecideRequestError;

    fn try_from(mut value: raw_probe::DecideRequest) -> Result<Self, Self::Error> {
        require_text(&value.request_id, "request_id")?;
        require_text(&value.idempotency_key, "idempotency_key")?;
        require_text(&value.source_handle, "source_handle")?;
        if value.filter_epoch.is_none() {
            return Err(DecideRequestError::MissingScalar("filter_epoch"));
        }
        if !matches!(value.remaining_transport_budget_nanoseconds, Some(1..)) {
            return Err(DecideRequestError::MissingTransportBudget);
        }
        let producer = ProducerEvent::try_from(
            value
                .producer_event
                .take()
                .ok_or(DecideRequestError::MissingField("producer_event"))?,
        )
        .map_err(DecideRequestError::InvalidProducer)?;
        if value.filter_epoch != Some(producer.filter_epoch().value()) {
            return Err(DecideRequestError::FilterEpochMismatch);
        }
        value.producer_event = Some(producer.0);
        Ok(Self(value))
    }
}

/// Probe behavior when Sentinel times out or returns DEFER.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum UnresolvedResolution {
    /// At least one matched Specification is fail-closed.
    Deny,
    /// Every matched Specification is fail-open.
    PermitAndAudit,
}

/// Validates a deferred response and applies deterministic per-Spec fail-mode aggregation.
///
/// # Errors
///
/// Returns an error unless the response contains exactly one deferred record for every
/// matched active-enforcement Specification, with the Filter's declared fail mode and a
/// named unresolved reason.
pub fn aggregate_unresolved(
    filter: &EventFilter,
    matched_specifications: &BTreeSet<(&str, &str)>,
    response: &raw_probe::DecideResponse,
) -> Result<UnresolvedResolution, DecideRequestError> {
    require_text(&response.request_id, "decide_response.request_id")?;
    if parse_action(response.action)? != raw_probe::DecisionAction::Defer {
        return Err(DecideRequestError::UnresolvedActionMismatch);
    }
    if response.specifications.is_empty() {
        return Err(DecideRequestError::MissingUnresolvedDecisions);
    }

    let expected: BTreeSet<(String, String)> = matched_specifications
        .iter()
        .map(|(id, version)| ((*id).to_owned(), (*version).to_owned()))
        .collect();
    for identity in &expected {
        let Some(specification) = filter.specifications().iter().find(|candidate| {
            candidate.specification_id == identity.0
                && candidate.specification_version == identity.1
        }) else {
            return Err(DecideRequestError::IrrelevantSpecificationDecision);
        };
        if specification.readiness != raw_model::Readiness::Active as i32
            || specification.evaluation_mode != raw_model::EvaluationMode::Enforce as i32
        {
            return Err(DecideRequestError::IrrelevantSpecificationDecision);
        }
    }

    let mut fail_closed = false;
    let mut actual = BTreeSet::new();
    for specification in &response.specifications {
        require_text(
            &specification.specification_id,
            "specification_decision.specification_id",
        )?;
        require_text(
            &specification.specification_version,
            "specification_decision.specification_version",
        )?;
        let identity = (
            specification.specification_id.clone(),
            specification.specification_version.clone(),
        );
        if !actual.insert(identity.clone()) {
            return Err(DecideRequestError::DuplicateValue(
                "specification_decisions",
            ));
        }
        if !expected.contains(&identity) {
            return Err(DecideRequestError::IrrelevantSpecificationDecision);
        }
        let filter_specification = filter
            .specifications()
            .iter()
            .find(|candidate| {
                candidate.specification_id == identity.0
                    && candidate.specification_version == identity.1
            })
            .ok_or(DecideRequestError::IrrelevantSpecificationDecision)?;
        let action = parse_action(specification.action)?;
        let fail_mode = parse_fail_mode(specification.fail_mode)?;
        if action != raw_probe::DecisionAction::Defer {
            return Err(DecideRequestError::UnresolvedActionMismatch);
        }
        let reason = specification
            .unresolved_reason
            .ok_or(DecideRequestError::MissingUnresolvedReason)?;
        match raw_probe::UnresolvedReason::try_from(reason) {
            Ok(raw_probe::UnresolvedReason::Unspecified) => {
                return Err(DecideRequestError::UnspecifiedEnum("unresolved_reason"))
            }
            Err(_) => {
                return Err(DecideRequestError::UnknownEnum {
                    field: "unresolved_reason",
                    value: reason,
                })
            }
            Ok(_) => {}
        }
        if specification.fail_mode != filter_specification.fail_mode {
            return Err(DecideRequestError::DecisionFailModeMismatch);
        }
        fail_closed |= fail_mode == raw_model::FailMode::Closed;
    }
    if actual != expected {
        return Err(DecideRequestError::IncompleteSpecificationDecisions);
    }
    Ok(if fail_closed {
        UnresolvedResolution::Deny
    } else {
        UnresolvedResolution::PermitAndAudit
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `DecideRequestError` variant paired with the exact `Display` output it must produce.
    ///
    /// Carried over from the single-error table this type was split out of (#65), verbatim
    /// but for one deliberate divergence: `MissingTransportBudget` names the request's
    /// transport budget instead of repeating the Event Filter's enforcement-budget wording.
    /// `EventFilterError`'s row keeps the pre-split string, so the two now differ on purpose.
    fn error_display_cases() -> Vec<(DecideRequestError, &'static str)> {
        vec![
            (
                DecideRequestError::MissingScalar("field"),
                "required scalar is missing: field",
            ),
            (
                DecideRequestError::MissingField("field"),
                "required message is missing: field",
            ),
            (
                DecideRequestError::UnknownEnum {
                    field: "field",
                    value: 7,
                },
                "unknown field enum value: 7",
            ),
            (
                DecideRequestError::UnspecifiedEnum("field"),
                "field must not be unspecified",
            ),
            (
                DecideRequestError::DuplicateValue("field"),
                "duplicate value in set-like field: field",
            ),
            (
                DecideRequestError::MissingTransportBudget,
                "request requires a nonzero transport budget",
            ),
            (
                DecideRequestError::FilterEpochMismatch,
                "request and producer Filter epochs differ",
            ),
            (
                DecideRequestError::MissingUnresolvedReason,
                "deferred decision requires an unresolved reason",
            ),
            (
                DecideRequestError::MissingUnresolvedDecisions,
                "deferred response requires at least one unresolved Specification",
            ),
            (
                DecideRequestError::UnresolvedActionMismatch,
                "unresolved response and every Specification decision must defer",
            ),
            (
                DecideRequestError::IrrelevantSpecificationDecision,
                "unresolved decision is not a matched active-enforcement Specification",
            ),
            (
                DecideRequestError::DecisionFailModeMismatch,
                "unresolved decision fail mode differs from the active Filter",
            ),
            (
                DecideRequestError::IncompleteSpecificationDecisions,
                "unresolved response does not cover every matched active-enforcement Specification",
            ),
            (
                DecideRequestError::InvalidProducer(crate::EventError::SelfCausalPredecessor),
                "invalid producer event: event must not be its own causal predecessor",
            ),
        ]
    }

    #[test]
    fn error_display_is_stable() {
        let cases = error_display_cases();
        assert_eq!(
            cases.len(),
            14,
            "every DecideRequestError variant must be pinned"
        );
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected, "{error:?}");
        }
    }

    #[test]
    fn error_source_is_exposed_only_for_wrapped_errors() {
        for (error, _) in error_display_cases() {
            let wraps_event_error = matches!(error, DecideRequestError::InvalidProducer(_));
            assert_eq!(error.source().is_some(), wraps_event_error, "{error:?}");
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
    fn any_fail_closed_unresolved_specification_denies() {
        let mut raw_filter = active_filter(vec![watermark()]);
        let mut closed = raw_filter.specifications[0].clone();
        closed.specification_id = "closed".to_owned();
        closed.fail_mode = raw_model::FailMode::Closed as i32;
        raw_filter.specifications.push(closed);
        let filter = EventFilter::try_from(raw_filter).expect("valid active Filter");
        let specification = |id: &str, fail_mode| raw_probe::SpecificationDecision {
            specification_id: id.to_owned(),
            specification_version: "v1".to_owned(),
            action: raw_probe::DecisionAction::Defer as i32,
            fail_mode,
            unresolved_reason: Some(raw_probe::UnresolvedReason::Timeout as i32),
        };
        let matched = BTreeSet::from([("closed", "v1"), ("spec", "v1")]);
        let response = raw_probe::DecideResponse {
            request_id: "request".to_owned(),
            action: raw_probe::DecisionAction::Defer as i32,
            specifications: vec![
                specification("closed", raw_model::FailMode::Closed as i32),
                specification("spec", raw_model::FailMode::Open as i32),
            ],
        };
        assert_eq!(
            aggregate_unresolved(&filter, &matched, &response),
            Ok(UnresolvedResolution::Deny)
        );

        let mut reversed = response;
        reversed.specifications.reverse();
        assert_eq!(
            aggregate_unresolved(&filter, &matched, &reversed),
            Ok(UnresolvedResolution::Deny)
        );

        let mut incomplete = reversed;
        incomplete.specifications.pop();
        assert_eq!(
            aggregate_unresolved(&filter, &matched, &incomplete),
            Err(DecideRequestError::IncompleteSpecificationDecisions)
        );
    }

    fn unresolved_response(
        specification_id: &str,
        action: raw_probe::DecisionAction,
    ) -> raw_probe::DecideResponse {
        raw_probe::DecideResponse {
            request_id: "request".to_owned(),
            action: raw_probe::DecisionAction::Defer as i32,
            specifications: vec![raw_probe::SpecificationDecision {
                specification_id: specification_id.to_owned(),
                specification_version: "v1".to_owned(),
                action: action as i32,
                fail_mode: raw_model::FailMode::Open as i32,
                unresolved_reason: (action == raw_probe::DecisionAction::Defer)
                    .then_some(raw_probe::UnresolvedReason::EvidenceGap as i32),
            }],
        }
    }

    #[test]
    fn unresolved_aggregation_rejects_empty_duplicate_settled_and_irrelevant_records() {
        let filter =
            EventFilter::try_from(active_filter(vec![watermark()])).expect("valid active Filter");
        let matched = BTreeSet::from([("spec", "v1")]);

        let mut empty = unresolved_response("spec", raw_probe::DecisionAction::Defer);
        empty.specifications.clear();
        assert_eq!(
            aggregate_unresolved(&filter, &matched, &empty),
            Err(DecideRequestError::MissingUnresolvedDecisions)
        );

        let mut duplicate = unresolved_response("spec", raw_probe::DecisionAction::Defer);
        duplicate
            .specifications
            .push(duplicate.specifications[0].clone());
        assert_eq!(
            aggregate_unresolved(&filter, &matched, &duplicate),
            Err(DecideRequestError::DuplicateValue(
                "specification_decisions"
            ))
        );

        let settled = unresolved_response("spec", raw_probe::DecisionAction::Permit);
        assert_eq!(
            aggregate_unresolved(&filter, &matched, &settled),
            Err(DecideRequestError::UnresolvedActionMismatch)
        );

        let irrelevant = unresolved_response("other", raw_probe::DecisionAction::Defer);
        assert_eq!(
            aggregate_unresolved(&filter, &matched, &irrelevant),
            Err(DecideRequestError::IrrelevantSpecificationDecision)
        );

        let mut settled_response = unresolved_response("spec", raw_probe::DecisionAction::Defer);
        settled_response.action = raw_probe::DecisionAction::Permit as i32;
        assert_eq!(
            aggregate_unresolved(&filter, &matched, &settled_response),
            Err(DecideRequestError::UnresolvedActionMismatch)
        );
    }

    #[test]
    fn unresolved_aggregation_rejects_omitted_or_non_enforcing_specs_and_fail_mode_drift() {
        let filter =
            EventFilter::try_from(active_filter(vec![watermark()])).expect("valid active Filter");
        let response = unresolved_response("spec", raw_probe::DecisionAction::Defer);
        assert_eq!(
            aggregate_unresolved(&filter, &BTreeSet::from([("spec", "v1")]), &response),
            Ok(UnresolvedResolution::PermitAndAudit)
        );

        assert_eq!(
            aggregate_unresolved(&filter, &BTreeSet::from([("missing", "v1")]), &response,),
            Err(DecideRequestError::IrrelevantSpecificationDecision)
        );

        let mut drifted = response.clone();
        drifted.specifications[0].fail_mode = raw_model::FailMode::Closed as i32;
        assert_eq!(
            aggregate_unresolved(&filter, &BTreeSet::from([("spec", "v1")]), &drifted),
            Err(DecideRequestError::DecisionFailModeMismatch)
        );

        let mut raw_detect = active_filter(vec![watermark()]);
        raw_detect.specifications[0].evaluation_mode = raw_model::EvaluationMode::Detect as i32;
        let detect = EventFilter::try_from(raw_detect).expect("valid detect Filter");
        assert_eq!(
            aggregate_unresolved(&detect, &BTreeSet::from([("spec", "v1")]), &response),
            Err(DecideRequestError::IrrelevantSpecificationDecision)
        );
    }

    /// A warming Specification in Shadow can neither be denied nor block a caller.
    ///
    /// ADR-0017's "a new Specification starts in a warming Shadow state" is only worth
    /// anything if the Probe path refuses to act on one, and it is refused on **both**
    /// axes independently: `readiness != Active` and `evaluation_mode != Enforce` each
    /// suffice on their own. Pinned as a pair because the two are separately reachable —
    /// `sentinel_compile` today emits `Active` with `Shadow`, and the lifecycle work that
    /// fixes the readiness axis must not be able to open the enforcement path while doing
    /// it.
    ///
    /// The budget half matters as much as the decision half: a Specification that
    /// contributed a blocking budget would make a caller wait on a Verdict that can never
    /// deny, which is enforcement's cost with none of its value.
    #[test]
    fn a_warming_shadow_specification_neither_denies_nor_blocks() {
        let response = unresolved_response("spec", raw_probe::DecisionAction::Defer);
        let matched = BTreeSet::from([("spec", "v1")]);

        for (readiness, evaluation) in [
            (
                raw_model::Readiness::Warming,
                raw_model::EvaluationMode::Shadow,
            ),
            (
                raw_model::Readiness::Active,
                raw_model::EvaluationMode::Shadow,
            ),
        ] {
            let mut raw = active_filter(vec![watermark()]);
            raw.specifications[0].readiness = readiness as i32;
            raw.specifications[0].evaluation_mode = evaluation as i32;
            let filter = EventFilter::try_from(raw).expect("a shadow Filter is valid");

            assert_eq!(
                aggregate_unresolved(&filter, &matched, &response),
                Err(DecideRequestError::IrrelevantSpecificationDecision),
                "{readiness:?}/{evaluation:?}"
            );
            assert_eq!(
                filter.effective_budget_nanoseconds(100, &BTreeSet::from(["spec"])),
                100,
                "{readiness:?}/{evaluation:?} must contribute no blocking budget"
            );
        }
    }

    /// A minimal valid raw request: the producer evidence acknowledges the
    /// same epoch the request declares, which is the invariant `filter_epoch`
    /// exists to expose.
    fn raw_request(
        filter_epoch: Option<u64>,
        producer_epoch: Option<u64>,
    ) -> raw_probe::DecideRequest {
        raw_probe::DecideRequest {
            request_id: "request".to_owned(),
            idempotency_key: "key".to_owned(),
            source_handle: "probe".to_owned(),
            filter_epoch,
            producer_event: Some(raw_model::ProducerEvent {
                id: "trace:tool-call:requested".to_owned(),
                sequence: Some(raw_model::SequenceCoordinate {
                    epoch: 1,
                    sequence: 1,
                }),
                schema_version: "schema".to_owned(),
                acknowledged_filter_epoch: producer_epoch,
                kind: "tool.requested".to_owned(),
                occurrence_time: Some(raw_model::OccurrenceTime {
                    clock_domain_id: "producer".to_owned(),
                    nanoseconds: Some(raw_model::Int128::default()),
                    uncertainty_nanoseconds: 0,
                }),
                attributes: Vec::new(),
                claimed_capabilities: Vec::new(),
                claimed_sensitivity: raw_model::Sensitivity::Internal as i32,
                causal_predecessor_ids: Vec::new(),
            }),
            remaining_transport_budget_nanoseconds: Some(1_000),
        }
    }

    #[test]
    fn request_accessors_return_the_validated_values() {
        let request = DecideRequest::try_from(raw_request(Some(9), Some(9)))
            .expect("a valid request validates");
        assert_eq!(request.request_id(), "request");
        assert_eq!(request.filter_epoch(), FilterEpoch::new(9));
        assert_eq!(request.remaining_transport_budget_nanoseconds(), 1_000);
        let producer = request.producer();
        assert_eq!(producer.id().as_str(), "trace:tool-call:requested");
        assert_eq!(producer.kind().as_str(), "tool.requested");
        assert!(
            producer.causal_predecessors().is_empty(),
            "the fixture producer declares no predecessors"
        );

        assert_eq!(
            DecideRequest::try_from(raw_request(Some(8), Some(9))),
            Err(DecideRequestError::FilterEpochMismatch),
            "request and producer epochs must agree before the accessor can expose one"
        );
    }
}
