// @generated
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct DecideRequest {
    #[prost(string, tag="1")]
    pub request_id: ::prost::alloc::string::String,
    #[prost(string, tag="2")]
    pub idempotency_key: ::prost::alloc::string::String,
    #[prost(string, tag="3")]
    pub source_handle: ::prost::alloc::string::String,
    #[prost(uint64, optional, tag="4")]
    pub filter_epoch: ::core::option::Option<u64>,
    #[prost(message, optional, tag="5")]
    pub producer_event: ::core::option::Option<super::super::model::v1::ProducerEvent>,
    /// Relative budget remaining at this hop. Each hop decrements it using a local
    /// monotonic clock; it is never an absolute deadline from another clock domain.
    #[prost(uint64, optional, tag="6")]
    pub remaining_transport_budget_nanoseconds: ::core::option::Option<u64>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SpecificationDecision {
    #[prost(string, tag="1")]
    pub specification_id: ::prost::alloc::string::String,
    #[prost(string, tag="2")]
    pub specification_version: ::prost::alloc::string::String,
    #[prost(enumeration="DecisionAction", tag="3")]
    pub action: i32,
    #[prost(enumeration="super::super::model::v1::FailMode", tag="4")]
    pub fail_mode: i32,
    #[prost(enumeration="UnresolvedReason", optional, tag="5")]
    pub unresolved_reason: ::core::option::Option<i32>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct DecideResponse {
    #[prost(string, tag="1")]
    pub request_id: ::prost::alloc::string::String,
    #[prost(enumeration="DecisionAction", tag="2")]
    pub action: i32,
    #[prost(message, repeated, tag="3")]
    pub specifications: ::prost::alloc::vec::Vec<SpecificationDecision>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum DecisionAction {
    Unspecified = 0,
    Permit = 1,
    Deny = 2,
    Defer = 3,
}
impl DecisionAction {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            DecisionAction::Unspecified => "DECISION_ACTION_UNSPECIFIED",
            DecisionAction::Permit => "DECISION_ACTION_PERMIT",
            DecisionAction::Deny => "DECISION_ACTION_DENY",
            DecisionAction::Defer => "DECISION_ACTION_DEFER",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "DECISION_ACTION_UNSPECIFIED" => Some(Self::Unspecified),
            "DECISION_ACTION_PERMIT" => Some(Self::Permit),
            "DECISION_ACTION_DENY" => Some(Self::Deny),
            "DECISION_ACTION_DEFER" => Some(Self::Defer),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum UnresolvedReason {
    Unspecified = 0,
    PrefixUncertainty = 1,
    ObservationalUncertainty = 2,
    EpistemicUncertainty = 3,
    EvidenceGap = 4,
    Timeout = 5,
}
impl UnresolvedReason {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            UnresolvedReason::Unspecified => "UNRESOLVED_REASON_UNSPECIFIED",
            UnresolvedReason::PrefixUncertainty => "UNRESOLVED_REASON_PREFIX_UNCERTAINTY",
            UnresolvedReason::ObservationalUncertainty => "UNRESOLVED_REASON_OBSERVATIONAL_UNCERTAINTY",
            UnresolvedReason::EpistemicUncertainty => "UNRESOLVED_REASON_EPISTEMIC_UNCERTAINTY",
            UnresolvedReason::EvidenceGap => "UNRESOLVED_REASON_EVIDENCE_GAP",
            UnresolvedReason::Timeout => "UNRESOLVED_REASON_TIMEOUT",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "UNRESOLVED_REASON_UNSPECIFIED" => Some(Self::Unspecified),
            "UNRESOLVED_REASON_PREFIX_UNCERTAINTY" => Some(Self::PrefixUncertainty),
            "UNRESOLVED_REASON_OBSERVATIONAL_UNCERTAINTY" => Some(Self::ObservationalUncertainty),
            "UNRESOLVED_REASON_EPISTEMIC_UNCERTAINTY" => Some(Self::EpistemicUncertainty),
            "UNRESOLVED_REASON_EVIDENCE_GAP" => Some(Self::EvidenceGap),
            "UNRESOLVED_REASON_TIMEOUT" => Some(Self::Timeout),
            _ => None,
        }
    }
}
// @@protoc_insertion_point(module)
