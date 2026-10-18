// @generated
/// A signed 128-bit integer split into two fixed-width words.
/// Its value is high * 2^64 + low, with high interpreted as signed two's complement.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Int128 {
    #[prost(sfixed64, tag="1")]
    pub high: i64,
    #[prost(fixed64, tag="2")]
    pub low: u64,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SourceIdentity {
    #[prost(string, tag="1")]
    pub monitored_application_id: ::prost::alloc::string::String,
    #[prost(string, tag="2")]
    pub source_id: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SequenceCoordinate {
    #[prost(uint64, tag="1")]
    pub epoch: u64,
    #[prost(uint64, tag="2")]
    pub sequence: u64,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct OccurrenceTime {
    #[prost(string, tag="1")]
    pub clock_domain_id: ::prost::alloc::string::String,
    #[prost(message, optional, tag="2")]
    pub nanoseconds: ::core::option::Option<Int128>,
    #[prost(uint64, tag="3")]
    pub uncertainty_nanoseconds: u64,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ObservationTime {
    #[prost(string, tag="1")]
    pub sentinel_clock_id: ::prost::alloc::string::String,
    #[prost(message, optional, tag="2")]
    pub nanoseconds: ::core::option::Option<Int128>,
}
/// Deployment and transport assurance assigned after producer input is received.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SourceAssurance {
    #[prost(message, optional, tag="1")]
    pub source: ::core::option::Option<SourceIdentity>,
    #[prost(enumeration="SourceTier", tag="2")]
    pub tier: i32,
    #[prost(enumeration="SourceCapability", repeated, tag="3")]
    pub effective_capabilities: ::prost::alloc::vec::Vec<i32>,
    #[prost(enumeration="Integrity", tag="4")]
    pub integrity: i32,
    #[prost(bytes="vec", tag="5")]
    pub authenticated_identity_evidence: ::prost::alloc::vec::Vec<u8>,
    #[prost(bytes="vec", tag="6")]
    pub integrity_evidence: ::prost::alloc::vec::Vec<u8>,
    #[prost(string, tag="7")]
    pub topology_id: ::prost::alloc::string::String,
}
/// Authored by a Probe; it cannot assert receiver-local identity or observation time.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ProducerCompletenessSignal {
    #[prost(uint64, tag="1")]
    pub sequence_epoch: u64,
    #[prost(uint64, optional, tag="2")]
    pub last_emitted: ::core::option::Option<u64>,
    #[prost(uint64, optional, tag="3")]
    pub complete_through: ::core::option::Option<u64>,
    #[prost(message, optional, tag="4")]
    pub producer_reported_at: ::core::option::Option<OccurrenceTime>,
    #[prost(uint64, optional, tag="5")]
    pub acknowledged_filter_epoch: ::core::option::Option<u64>,
    #[prost(enumeration="SourceActivityState", tag="6")]
    pub activity: i32,
    #[prost(message, optional, tag="7")]
    pub activity_since: ::core::option::Option<OccurrenceTime>,
}
/// Sentinel's canonical completeness evidence after transport/deployment verification.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SourceCompletenessSignal {
    #[prost(message, optional, tag="1")]
    pub producer_signal: ::core::option::Option<ProducerCompletenessSignal>,
    #[prost(message, optional, tag="2")]
    pub assurance: ::core::option::Option<SourceAssurance>,
    #[prost(message, optional, tag="3")]
    pub observed_at: ::core::option::Option<ObservationTime>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Watermark {
    #[prost(message, optional, tag="1")]
    pub source: ::core::option::Option<SourceIdentity>,
    #[prost(uint64, tag="2")]
    pub sequence_epoch: u64,
    #[prost(uint64, tag="3")]
    pub complete_through: u64,
    #[prost(message, optional, tag="4")]
    pub observed_at: ::core::option::Option<ObservationTime>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum SourceCapability {
    Unspecified = 0,
    ObserveBeforeEffect = 1,
    CausalEdges = 2,
    CompletenessSignals = 3,
    BoundedClockUncertainty = 4,
}
impl SourceCapability {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            SourceCapability::Unspecified => "SOURCE_CAPABILITY_UNSPECIFIED",
            SourceCapability::ObserveBeforeEffect => "SOURCE_CAPABILITY_OBSERVE_BEFORE_EFFECT",
            SourceCapability::CausalEdges => "SOURCE_CAPABILITY_CAUSAL_EDGES",
            SourceCapability::CompletenessSignals => "SOURCE_CAPABILITY_COMPLETENESS_SIGNALS",
            SourceCapability::BoundedClockUncertainty => "SOURCE_CAPABILITY_BOUNDED_CLOCK_UNCERTAINTY",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "SOURCE_CAPABILITY_UNSPECIFIED" => Some(Self::Unspecified),
            "SOURCE_CAPABILITY_OBSERVE_BEFORE_EFFECT" => Some(Self::ObserveBeforeEffect),
            "SOURCE_CAPABILITY_CAUSAL_EDGES" => Some(Self::CausalEdges),
            "SOURCE_CAPABILITY_COMPLETENESS_SIGNALS" => Some(Self::CompletenessSignals),
            "SOURCE_CAPABILITY_BOUNDED_CLOCK_UNCERTAINTY" => Some(Self::BoundedClockUncertainty),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum Sensitivity {
    Unspecified = 0,
    Public = 1,
    Internal = 2,
    Confidential = 3,
    Restricted = 4,
}
impl Sensitivity {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            Sensitivity::Unspecified => "SENSITIVITY_UNSPECIFIED",
            Sensitivity::Public => "SENSITIVITY_PUBLIC",
            Sensitivity::Internal => "SENSITIVITY_INTERNAL",
            Sensitivity::Confidential => "SENSITIVITY_CONFIDENTIAL",
            Sensitivity::Restricted => "SENSITIVITY_RESTRICTED",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "SENSITIVITY_UNSPECIFIED" => Some(Self::Unspecified),
            "SENSITIVITY_PUBLIC" => Some(Self::Public),
            "SENSITIVITY_INTERNAL" => Some(Self::Internal),
            "SENSITIVITY_CONFIDENTIAL" => Some(Self::Confidential),
            "SENSITIVITY_RESTRICTED" => Some(Self::Restricted),
            _ => None,
        }
    }
}
/// The data class an Event belongs to (ADR-0029). Monitored-Tenant data is governed by ADR-0026;
/// Corpus data is donated, sanitized evidence under a DUA and never receives tenant scoping.
/// The two classes have different retention, access semantics and consumers, so an Event must
/// declare which it is before anything downstream may store or evaluate it. UNSPECIFIED is
/// rejected by validation: a classless Event is the silent-wrong failure ADR-0029 exists to
/// prevent, not a parse error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum DataClass {
    Unspecified = 0,
    Tenant = 1,
    Corpus = 2,
}
impl DataClass {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            DataClass::Unspecified => "DATA_CLASS_UNSPECIFIED",
            DataClass::Tenant => "DATA_CLASS_TENANT",
            DataClass::Corpus => "DATA_CLASS_CORPUS",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "DATA_CLASS_UNSPECIFIED" => Some(Self::Unspecified),
            "DATA_CLASS_TENANT" => Some(Self::Tenant),
            "DATA_CLASS_CORPUS" => Some(Self::Corpus),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum SourceTier {
    Unspecified = 0,
    Anchor = 1,
    Contributing = 2,
}
impl SourceTier {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            SourceTier::Unspecified => "SOURCE_TIER_UNSPECIFIED",
            SourceTier::Anchor => "SOURCE_TIER_ANCHOR",
            SourceTier::Contributing => "SOURCE_TIER_CONTRIBUTING",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "SOURCE_TIER_UNSPECIFIED" => Some(Self::Unspecified),
            "SOURCE_TIER_ANCHOR" => Some(Self::Anchor),
            "SOURCE_TIER_CONTRIBUTING" => Some(Self::Contributing),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum Integrity {
    Unspecified = 0,
    Unverified = 1,
    Authenticated = 2,
    TamperEvident = 3,
}
impl Integrity {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            Integrity::Unspecified => "INTEGRITY_UNSPECIFIED",
            Integrity::Unverified => "INTEGRITY_UNVERIFIED",
            Integrity::Authenticated => "INTEGRITY_AUTHENTICATED",
            Integrity::TamperEvident => "INTEGRITY_TAMPER_EVIDENT",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "INTEGRITY_UNSPECIFIED" => Some(Self::Unspecified),
            "INTEGRITY_UNVERIFIED" => Some(Self::Unverified),
            "INTEGRITY_AUTHENTICATED" => Some(Self::Authenticated),
            "INTEGRITY_TAMPER_EVIDENT" => Some(Self::TamperEvident),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum SourceActivityState {
    Unspecified = 0,
    Active = 1,
    IntentionallyIdle = 2,
}
impl SourceActivityState {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            SourceActivityState::Unspecified => "SOURCE_ACTIVITY_STATE_UNSPECIFIED",
            SourceActivityState::Active => "SOURCE_ACTIVITY_STATE_ACTIVE",
            SourceActivityState::IntentionallyIdle => "SOURCE_ACTIVITY_STATE_INTENTIONALLY_IDLE",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "SOURCE_ACTIVITY_STATE_UNSPECIFIED" => Some(Self::Unspecified),
            "SOURCE_ACTIVITY_STATE_ACTIVE" => Some(Self::Active),
            "SOURCE_ACTIVITY_STATE_INTENTIONALLY_IDLE" => Some(Self::IntentionallyIdle),
            _ => None,
        }
    }
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AttributeArray {
    #[prost(message, repeated, tag="1")]
    pub values: ::prost::alloc::vec::Vec<AttributeValue>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AttributeMap {
    #[prost(message, repeated, tag="1")]
    pub entries: ::prost::alloc::vec::Vec<AttributeEntry>,
}
/// One attribute in a map. The claimed_sensitivity is a per-attribute assertion by the
/// Probe (ADR-0026 §9); SENSITIVITY_UNSPECIFIED defers to ProducerEvent.claimed_sensitivity.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AttributeEntry {
    #[prost(string, tag="1")]
    pub key: ::prost::alloc::string::String,
    #[prost(message, optional, tag="2")]
    pub value: ::core::option::Option<AttributeValue>,
    #[prost(enumeration="Sensitivity", tag="3")]
    pub claimed_sensitivity: i32,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AttributeValue {
    #[prost(oneof="attribute_value::Value", tags="1, 2, 3, 4, 5, 6, 7")]
    pub value: ::core::option::Option<attribute_value::Value>,
}
/// Nested message and enum types in `AttributeValue`.
pub mod attribute_value {
    #[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Value {
        #[prost(string, tag="1")]
        StringValue(::prost::alloc::string::String),
        #[prost(bool, tag="2")]
        BoolValue(bool),
        #[prost(sint64, tag="3")]
        IntegerValue(i64),
        #[prost(double, tag="4")]
        DoubleValue(f64),
        #[prost(bytes, tag="5")]
        BytesValue(::prost::alloc::vec::Vec<u8>),
        #[prost(message, tag="6")]
        ArrayValue(super::AttributeArray),
        #[prost(message, tag="7")]
        MapValue(super::AttributeMap),
    }
}
/// Evidence asserted by a Probe. Source identity, tier, integrity, and observation time
/// are deliberately absent because the receiver owns those authority domains.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ProducerEvent {
    #[prost(string, tag="1")]
    pub id: ::prost::alloc::string::String,
    #[prost(message, optional, tag="2")]
    pub sequence: ::core::option::Option<SequenceCoordinate>,
    #[prost(string, tag="3")]
    pub schema_version: ::prost::alloc::string::String,
    #[prost(uint64, optional, tag="4")]
    pub acknowledged_filter_epoch: ::core::option::Option<u64>,
    #[prost(string, tag="5")]
    pub kind: ::prost::alloc::string::String,
    #[prost(message, optional, tag="6")]
    pub occurrence_time: ::core::option::Option<OccurrenceTime>,
    #[prost(message, repeated, tag="7")]
    pub attributes: ::prost::alloc::vec::Vec<AttributeEntry>,
    #[prost(enumeration="SourceCapability", repeated, tag="8")]
    pub claimed_capabilities: ::prost::alloc::vec::Vec<i32>,
    #[prost(enumeration="Sensitivity", tag="9")]
    pub claimed_sensitivity: i32,
    #[prost(string, repeated, tag="10")]
    pub causal_predecessor_ids: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
}
/// Canonical Event formed only by Sentinel after authenticating and enriching producer input.
/// tenant_id and deployment_id are Sentinel-assigned scoping (ADR-0026 §1, §10): the
/// isolation boundary and the Specification/Verdict scope. They are not producer-asserted.
/// deployment_id identifies the Deployment instance; SourceAssurance.topology_id references
/// the Topology (declared shape) of that Deployment.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Event {
    #[prost(message, optional, tag="1")]
    pub producer_event: ::core::option::Option<ProducerEvent>,
    #[prost(message, optional, tag="2")]
    pub assurance: ::core::option::Option<SourceAssurance>,
    #[prost(message, optional, tag="3")]
    pub observation_time: ::core::option::Option<ObservationTime>,
    #[prost(string, tag="4")]
    pub tenant_id: ::prost::alloc::string::String,
    #[prost(string, tag="5")]
    pub deployment_id: ::prost::alloc::string::String,
    /// Which data class this Event belongs to (ADR-0029). Set by the ingest boundary at
    /// observation time; corpus Events are never tenant-scoped and tenant Events are never
    /// corpus-class, and downstream storage must filter on this field rather than guess.
    #[prost(enumeration="DataClass", tag="6")]
    pub data_class: i32,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EventMatch {
    #[prost(string, repeated, tag="1")]
    pub event_kinds: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
    #[prost(string, repeated, tag="2")]
    pub projected_attribute_keys: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
    #[prost(enumeration="DeliveryMode", tag="3")]
    pub delivery_mode: i32,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SpecificationFilter {
    #[prost(string, tag="1")]
    pub specification_id: ::prost::alloc::string::String,
    #[prost(string, tag="2")]
    pub specification_version: ::prost::alloc::string::String,
    #[prost(message, optional, tag="3")]
    pub event_match: ::core::option::Option<EventMatch>,
    #[prost(enumeration="FailMode", tag="4")]
    pub fail_mode: i32,
    #[prost(uint64, optional, tag="5")]
    pub latency_budget_nanoseconds: ::core::option::Option<u64>,
    #[prost(enumeration="EvaluationMode", tag="6")]
    pub evaluation_mode: i32,
    #[prost(enumeration="Readiness", tag="7")]
    pub readiness: i32,
    #[prost(uint64, optional, tag="8")]
    pub target_filter_epoch: ::core::option::Option<u64>,
    #[prost(message, repeated, tag="9")]
    pub required_sources: ::prost::alloc::vec::Vec<SourceIdentity>,
    #[prost(message, repeated, tag="10")]
    pub activation_watermarks: ::prost::alloc::vec::Vec<Watermark>,
    #[prost(message, optional, tag="11")]
    pub enforcement_gates: ::core::option::Option<EnforcementGateEvidence>,
}
/// The five ADR-0023 gates, attested by the compiler/promotion boundary.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EnforcementGateEvidence {
    #[prost(bool, tag="1")]
    pub safety_property: bool,
    #[prost(bool, tag="2")]
    pub ask_and_block: bool,
    #[prost(bool, tag="3")]
    pub hot_path_only: bool,
    #[prost(bool, tag="4")]
    pub anchor_tier: bool,
    #[prost(bool, tag="5")]
    pub shadow_and_adversarial_validation_passed: bool,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct BaselineEvidenceFilter {
    #[prost(bool, tag="1")]
    pub source_health: bool,
    #[prost(bool, tag="2")]
    pub completeness_signals: bool,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EventFilter {
    #[prost(uint64, optional, tag="1")]
    pub epoch: ::core::option::Option<u64>,
    #[prost(message, repeated, tag="2")]
    pub specifications: ::prost::alloc::vec::Vec<SpecificationFilter>,
    #[prost(message, optional, tag="3")]
    pub baseline_evidence: ::core::option::Option<BaselineEvidenceFilter>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum FailMode {
    Unspecified = 0,
    Open = 1,
    Closed = 2,
}
impl FailMode {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            FailMode::Unspecified => "FAIL_MODE_UNSPECIFIED",
            FailMode::Open => "FAIL_MODE_OPEN",
            FailMode::Closed => "FAIL_MODE_CLOSED",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "FAIL_MODE_UNSPECIFIED" => Some(Self::Unspecified),
            "FAIL_MODE_OPEN" => Some(Self::Open),
            "FAIL_MODE_CLOSED" => Some(Self::Closed),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum EvaluationMode {
    Unspecified = 0,
    Shadow = 1,
    Detect = 2,
    Enforce = 3,
}
impl EvaluationMode {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            EvaluationMode::Unspecified => "EVALUATION_MODE_UNSPECIFIED",
            EvaluationMode::Shadow => "EVALUATION_MODE_SHADOW",
            EvaluationMode::Detect => "EVALUATION_MODE_DETECT",
            EvaluationMode::Enforce => "EVALUATION_MODE_ENFORCE",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "EVALUATION_MODE_UNSPECIFIED" => Some(Self::Unspecified),
            "EVALUATION_MODE_SHADOW" => Some(Self::Shadow),
            "EVALUATION_MODE_DETECT" => Some(Self::Detect),
            "EVALUATION_MODE_ENFORCE" => Some(Self::Enforce),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum Readiness {
    Unspecified = 0,
    Warming = 1,
    Active = 2,
}
impl Readiness {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            Readiness::Unspecified => "READINESS_UNSPECIFIED",
            Readiness::Warming => "READINESS_WARMING",
            Readiness::Active => "READINESS_ACTIVE",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "READINESS_UNSPECIFIED" => Some(Self::Unspecified),
            "READINESS_WARMING" => Some(Self::Warming),
            "READINESS_ACTIVE" => Some(Self::Active),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum DeliveryMode {
    Unspecified = 0,
    ShipAsync = 1,
    AskAndBlock = 2,
}
impl DeliveryMode {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            DeliveryMode::Unspecified => "DELIVERY_MODE_UNSPECIFIED",
            DeliveryMode::ShipAsync => "DELIVERY_MODE_SHIP_ASYNC",
            DeliveryMode::AskAndBlock => "DELIVERY_MODE_ASK_AND_BLOCK",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "DELIVERY_MODE_UNSPECIFIED" => Some(Self::Unspecified),
            "DELIVERY_MODE_SHIP_ASYNC" => Some(Self::ShipAsync),
            "DELIVERY_MODE_ASK_AND_BLOCK" => Some(Self::AskAndBlock),
            _ => None,
        }
    }
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct FilterEpochBySource {
    #[prost(message, optional, tag="1")]
    pub source: ::core::option::Option<SourceIdentity>,
    #[prost(uint64, optional, tag="2")]
    pub filter_epoch: ::core::option::Option<u64>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SourceGap {
    #[prost(message, optional, tag="1")]
    pub source: ::core::option::Option<SourceIdentity>,
    #[prost(uint64, tag="2")]
    pub sequence_epoch: u64,
    #[prost(uint64, tag="3")]
    pub first_missing: u64,
    #[prost(uint64, tag="4")]
    pub last_missing: u64,
    #[prost(string, tag="5")]
    pub effect: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SourceHealth {
    #[prost(message, optional, tag="1")]
    pub source: ::core::option::Option<SourceIdentity>,
    #[prost(bool, tag="2")]
    pub healthy: bool,
    #[prost(string, tag="3")]
    pub detail: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SourceCapabilityRecord {
    #[prost(message, optional, tag="1")]
    pub source: ::core::option::Option<SourceIdentity>,
    #[prost(enumeration="SourceCapability", repeated, tag="2")]
    pub capabilities: ::prost::alloc::vec::Vec<i32>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SourceTierRecord {
    #[prost(message, optional, tag="1")]
    pub source: ::core::option::Option<SourceIdentity>,
    #[prost(enumeration="SourceTier", tag="2")]
    pub tier: i32,
    /// Events from this source that anchor this Verdict. Empty means this source anchored no Event here; it does not downgrade the tier.
    #[prost(string, repeated, tag="3")]
    pub anchored_event_ids: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct UnresolvedCausalEdge {
    #[prost(string, tag="1")]
    pub event_id: ::prost::alloc::string::String,
    #[prost(string, tag="2")]
    pub missing_predecessor_id: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct ExtractionFact {
    #[prost(string, tag="1")]
    pub fact_id: ::prost::alloc::string::String,
    #[prost(bool, tag="2")]
    pub available: bool,
    #[prost(string, tag="3")]
    pub detail: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Uncertainty {
    #[prost(enumeration="InconclusiveReason", tag="1")]
    pub reason: i32,
    #[prost(string, tag="2")]
    pub detail: ::prost::alloc::string::String,
}
/// The explicit assurance downgrade when tier coverage is incomplete or no covered source anchors evidence.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SourceTierGap {
    /// Covered required sources classified as contributing-tier.
    #[prost(message, repeated, tag="1")]
    pub contributing_sources: ::prost::alloc::vec::Vec<SourceIdentity>,
    #[prost(string, tag="2")]
    pub detail: ::prost::alloc::string::String,
    /// Required sources without valid anchor evidence, including sources with no tier record.
    #[prost(message, repeated, tag="3")]
    pub unanchored_sources: ::prost::alloc::vec::Vec<SourceIdentity>,
}
/// An Event that participates in a closed, outcome-decisive violation witness.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct DecisiveEvent {
    #[prost(string, tag="1")]
    pub event_id: ::prost::alloc::string::String,
    #[prost(message, optional, tag="2")]
    pub source: ::core::option::Option<SourceIdentity>,
    #[prost(uint64, optional, tag="3")]
    pub sequence_epoch: ::core::option::Option<u64>,
    #[prost(uint64, optional, tag="4")]
    pub sequence_number: ::core::option::Option<u64>,
}
/// The exact open evidence a named proof rule establishes cannot change a violation.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct OutcomeInvariantLimitations {
    #[prost(message, repeated, tag="1")]
    pub gaps: ::prost::alloc::vec::Vec<SourceGap>,
    #[prost(message, repeated, tag="2")]
    pub unhealthy_sources: ::prost::alloc::vec::Vec<SourceIdentity>,
    #[prost(message, repeated, tag="3")]
    pub unresolved_causal_edges: ::prost::alloc::vec::Vec<UnresolvedCausalEdge>,
    #[prost(string, repeated, tag="4")]
    pub unavailable_extraction_fact_ids: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
    #[prost(enumeration="InconclusiveReason", repeated, tag="5")]
    pub uncertainty: ::prost::alloc::vec::Vec<i32>,
}
/// A named and versioned proof over a decisive witness and every open limitation.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct DecisiveViolationBasis {
    #[prost(message, repeated, tag="1")]
    pub witness_events: ::prost::alloc::vec::Vec<DecisiveEvent>,
    #[prost(string, repeated, tag="2")]
    pub required_extraction_fact_ids: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
    #[prost(string, tag="3")]
    pub invariance_proof_rule_id: ::prost::alloc::string::String,
    #[prost(string, tag="4")]
    pub invariance_proof_rule_version: ::prost::alloc::string::String,
    #[prost(message, optional, tag="5")]
    pub outcome_invariant_limitations: ::core::option::Option<OutcomeInvariantLimitations>,
}
/// Presence asserts that the full required evidence set is closed at settlement.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct CompleteEvidenceBasis {
}
/// A Violate settled by watermark-complete silence of required anchor-tier
/// source(s), not by witness Events (ADR-0033). Mutually exclusive with
/// DecisiveViolationBasis.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct WatermarkAnchoredViolationBasis {
    /// Anchor-tier source(s) whose silence, through the named Watermark, witnesses
    /// the absence.
    #[prost(message, repeated, tag="1")]
    pub silent_anchor_sources: ::prost::alloc::vec::Vec<SourceIdentity>,
    /// Completeness coordinate per source: the Watermark up to which each is
    /// complete, healthy, and emitted nothing for this Binding.
    #[prost(message, repeated, tag="2")]
    pub silence_through_watermarks: ::prost::alloc::vec::Vec<Watermark>,
    #[prost(string, tag="3")]
    pub invariance_proof_rule_id: ::prost::alloc::string::String,
    #[prost(string, tag="4")]
    pub invariance_proof_rule_version: ::prost::alloc::string::String,
    #[prost(message, optional, tag="5")]
    pub outcome_invariant_limitations: ::core::option::Option<OutcomeInvariantLimitations>,
}
/// The outcome-specific evidence basis that justifies an authoritative conclusion.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct SettlementBasis {
    #[prost(oneof="settlement_basis::Basis", tags="1, 2, 3")]
    pub basis: ::core::option::Option<settlement_basis::Basis>,
}
/// Nested message and enum types in `SettlementBasis`.
pub mod settlement_basis {
    #[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum Basis {
        #[prost(message, tag="1")]
        CompleteEvidence(super::CompleteEvidenceBasis),
        #[prost(message, tag="2")]
        DecisiveViolation(super::DecisiveViolationBasis),
        #[prost(message, tag="3")]
        WatermarkAnchoredViolation(super::WatermarkAnchoredViolationBasis),
    }
}
/// The evidence conditions inseparable from a Verdict (ADR-0017).
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct AssuranceEnvelope {
    #[prost(string, tag="1")]
    pub specification_id: ::prost::alloc::string::String,
    #[prost(string, tag="2")]
    pub specification_version: ::prost::alloc::string::String,
    #[prost(string, repeated, tag="3")]
    pub schema_versions: ::prost::alloc::vec::Vec<::prost::alloc::string::String>,
    #[prost(message, repeated, tag="4")]
    pub filter_epochs: ::prost::alloc::vec::Vec<FilterEpochBySource>,
    #[prost(message, repeated, tag="5")]
    pub watermarks: ::prost::alloc::vec::Vec<Watermark>,
    #[prost(message, repeated, tag="6")]
    pub required_sources: ::prost::alloc::vec::Vec<SourceIdentity>,
    #[prost(message, repeated, tag="7")]
    pub observed_sources: ::prost::alloc::vec::Vec<SourceIdentity>,
    #[prost(message, repeated, tag="8")]
    pub gaps: ::prost::alloc::vec::Vec<SourceGap>,
    #[prost(message, repeated, tag="9")]
    pub source_health: ::prost::alloc::vec::Vec<SourceHealth>,
    #[prost(message, repeated, tag="10")]
    pub capabilities: ::prost::alloc::vec::Vec<SourceCapabilityRecord>,
    #[prost(message, repeated, tag="11")]
    pub source_tiers_and_anchors: ::prost::alloc::vec::Vec<SourceTierRecord>,
    #[prost(message, repeated, tag="12")]
    pub unresolved_causal_edges: ::prost::alloc::vec::Vec<UnresolvedCausalEdge>,
    #[prost(message, repeated, tag="13")]
    pub extraction_facts: ::prost::alloc::vec::Vec<ExtractionFact>,
    #[prost(message, repeated, tag="14")]
    pub uncertainty: ::prost::alloc::vec::Vec<Uncertainty>,
    #[prost(enumeration="LateDataPolicy", tag="15")]
    pub late_data_policy: i32,
    #[prost(enumeration="TierSettlement", tag="16")]
    pub hot_path: i32,
    #[prost(enumeration="TierSettlement", tag="17")]
    pub cold_path: i32,
    #[prost(message, repeated, tag="18")]
    pub hot_settlement_watermarks: ::prost::alloc::vec::Vec<Watermark>,
    #[prost(message, repeated, tag="19")]
    pub cold_settlement_watermarks: ::prost::alloc::vec::Vec<Watermark>,
    #[prost(message, optional, tag="20")]
    pub source_tier_gap: ::core::option::Option<SourceTierGap>,
    #[prost(message, optional, tag="21")]
    pub settlement_basis: ::core::option::Option<SettlementBasis>,
    /// Discriminator. UNSPECIFIED is rejected at the model boundary.
    #[prost(enumeration="ActivationBarrierStatus", tag="22")]
    pub activation_barrier: i32,
    /// Specification-level target Filter epoch. Unset on NOT_DECLARED and on the
    /// historical #156 PENDING residual (NoEvidence / silent sources whose
    /// detail never interpolated a target). CLEARED requires this field.
    /// PENDING may leave it unset. Do not treat 0 as a missing-target sentinel:
    /// Filter epoch 0 is a real epoch.
    #[prost(uint64, optional, tag="23")]
    pub target_filter_epoch: ::core::option::Option<u64>,
    /// Raw-Event retention horizon in effect at settlement (ADR-0026 §6), in days.
    /// Absence is rejected at the model boundary for new construction; Verdict::decode
    /// recovers historical pre-#206 blobs (codec backfill to the platform default of
    /// 30 — historical recovery, not an export default). Validated range is 7..=365.
    /// Sentinel Events are not the EU AI Act Art. 12 automatically generated log; the
    /// 30-day platform default sits below the six-month Art. 19 / 26(6) deployer floor
    /// on purpose and must not be silently raised to fake that floor.
    #[prost(uint32, optional, tag="24")]
    pub raw_event_retention_days: ::core::option::Option<u32>,
    /// Forecast-at-settle inventory of attribute paths claimed / scheduled for
    /// post-settle redaction among attributes present on this Verdict's evidence
    /// (ADR-0026 §7; issue #244). Not "after the worker mutated Events": the sealed
    /// Verdict is immutable; a later redact worker mutates retained Events and writes
    /// §8 residue elsewhere — it does not rewrite this list.
    ///
    /// Empty (`\[\]`) means only that no redacted-attribute claims are recorded on this
    /// envelope. Proto3 cannot distinguish absence from empty. Do not infer that
    /// redaction completed, was a noop, or that a worker has or has not run.
    /// Value hashes / binding residue stay on the ADR-0026 §8 / evidence-extract
    /// surface — this message carries no hashes.
    ///
    /// Model validation rejects a non-empty list when redaction_policy is None.
    /// Duplicate (event_id, attribute_path) pairs and blank id/path are rejected.
    /// Canonical order is (event_id, attribute_path). Core must not invent entries.
    #[prost(message, repeated, tag="29")]
    pub redacted_attributes: ::prost::alloc::vec::Vec<RedactedAttributeRef>,
    /// Evidence-extract retention at settlement (ADR-0026 §6 / §8; bounds #243). Entire
    /// oneof unset is a defect at the model boundary. Configured days must be in 1..=365
    /// (shape-analogous to raw-Event bounds; 1..=6 remain legal). The Tenant-config /
    /// store-enabled default is 90 days when the configured arm is chosen — not a Core
    /// construction default while the store is undeployed, and not a Verdict::decode
    /// backfill value. NotDeployed is the truthful claim when this deployment operates
    /// no evidence-extract store — never encode that as days == 0. Verdict::decode
    /// backfills NotDeployed on historical blobs lacking this oneof. There is no
    /// evidence_extract ≤ raw_event coupling on the same envelope.
    #[prost(oneof="assurance_envelope::EvidenceExtractRetention", tags="25, 27")]
    pub evidence_extract_retention: ::core::option::Option<assurance_envelope::EvidenceExtractRetention>,
    /// Redaction policy in effect at settlement (ADR-0026 §7). Entire oneof unset is a
    /// defect at the model boundary. Configured version must be non-blank; None is the
    /// truthful claim when this deployment has no redaction policy — never encode that
    /// as an empty string. Verdict::decode backfills None on historical blobs lacking
    /// this oneof. The version/None claim is separate from the path inventory below.
    #[prost(oneof="assurance_envelope::RedactionPolicy", tags="26, 28")]
    pub redaction_policy: ::core::option::Option<assurance_envelope::RedactionPolicy>,
}
/// Nested message and enum types in `AssuranceEnvelope`.
pub mod assurance_envelope {
    /// Evidence-extract retention at settlement (ADR-0026 §6 / §8; bounds #243). Entire
    /// oneof unset is a defect at the model boundary. Configured days must be in 1..=365
    /// (shape-analogous to raw-Event bounds; 1..=6 remain legal). The Tenant-config /
    /// store-enabled default is 90 days when the configured arm is chosen — not a Core
    /// construction default while the store is undeployed, and not a Verdict::decode
    /// backfill value. NotDeployed is the truthful claim when this deployment operates
    /// no evidence-extract store — never encode that as days == 0. Verdict::decode
    /// backfills NotDeployed on historical blobs lacking this oneof. There is no
    /// evidence_extract ≤ raw_event coupling on the same envelope.
    #[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum EvidenceExtractRetention {
        #[prost(uint32, tag="25")]
        EvidenceExtractRetentionDays(u32),
        #[prost(message, tag="27")]
        EvidenceExtractNotDeployed(super::EvidenceExtractRetentionNotDeployed),
    }
    /// Redaction policy in effect at settlement (ADR-0026 §7). Entire oneof unset is a
    /// defect at the model boundary. Configured version must be non-blank; None is the
    /// truthful claim when this deployment has no redaction policy — never encode that
    /// as an empty string. Verdict::decode backfills None on historical blobs lacking
    /// this oneof. The version/None claim is separate from the path inventory below.
    #[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Oneof)]
    pub enum RedactionPolicy {
        #[prost(string, tag="26")]
        RedactionPolicyVersion(::prost::alloc::string::String),
        #[prost(message, tag="28")]
        RedactionPolicyNone(super::RedactionPolicyNone),
    }
}
/// Presence asserts that this deployment operates no evidence-extract store.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct EvidenceExtractRetentionNotDeployed {
}
/// Presence asserts that this deployment has no redaction policy configured.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RedactionPolicyNone {
}
/// One claimed / scheduled redaction binding under forecast-at-settle (#244).
/// attribute_path is an opaque Tenant-policy key — not a parsed redaction DSL.
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RedactedAttributeRef {
    /// non-blank
    #[prost(string, tag="1")]
    pub event_id: ::prost::alloc::string::String,
    /// non-blank; opaque Tenant-policy key (no DSL)
    #[prost(string, tag="2")]
    pub attribute_path: ::prost::alloc::string::String,
}
#[allow(clippy::derive_partial_eq_without_eq)]
#[derive(Clone, PartialEq, ::prost::Message)]
pub struct Verdict {
    #[prost(string, tag="1")]
    pub id: ::prost::alloc::string::String,
    #[prost(enumeration="VerdictOutcome", tag="2")]
    pub outcome: i32,
    #[prost(enumeration="InconclusiveReason", optional, tag="3")]
    pub inconclusive_reason: ::core::option::Option<i32>,
    #[prost(enumeration="VerdictAuthority", tag="4")]
    pub authority: i32,
    #[prost(message, optional, tag="5")]
    pub assurance: ::core::option::Option<AssuranceEnvelope>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum VerdictOutcome {
    Unspecified = 0,
    Satisfy = 1,
    Violate = 2,
    Inconclusive = 3,
}
impl VerdictOutcome {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            VerdictOutcome::Unspecified => "VERDICT_OUTCOME_UNSPECIFIED",
            VerdictOutcome::Satisfy => "VERDICT_OUTCOME_SATISFY",
            VerdictOutcome::Violate => "VERDICT_OUTCOME_VIOLATE",
            VerdictOutcome::Inconclusive => "VERDICT_OUTCOME_INCONCLUSIVE",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "VERDICT_OUTCOME_UNSPECIFIED" => Some(Self::Unspecified),
            "VERDICT_OUTCOME_SATISFY" => Some(Self::Satisfy),
            "VERDICT_OUTCOME_VIOLATE" => Some(Self::Violate),
            "VERDICT_OUTCOME_INCONCLUSIVE" => Some(Self::Inconclusive),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum InconclusiveReason {
    Unspecified = 0,
    PrefixUncertainty = 1,
    ObservationalUncertainty = 2,
    EpistemicUncertainty = 3,
    /// Complete, healthy, silent required anchor-tier source, but the Specification
    /// needs a presence that silence cannot confirm (ADR-0033).
    AnchorSilence = 5,
}
impl InconclusiveReason {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            InconclusiveReason::Unspecified => "INCONCLUSIVE_REASON_UNSPECIFIED",
            InconclusiveReason::PrefixUncertainty => "INCONCLUSIVE_REASON_PREFIX_UNCERTAINTY",
            InconclusiveReason::ObservationalUncertainty => "INCONCLUSIVE_REASON_OBSERVATIONAL_UNCERTAINTY",
            InconclusiveReason::EpistemicUncertainty => "INCONCLUSIVE_REASON_EPISTEMIC_UNCERTAINTY",
            InconclusiveReason::AnchorSilence => "INCONCLUSIVE_REASON_ANCHOR_SILENCE",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "INCONCLUSIVE_REASON_UNSPECIFIED" => Some(Self::Unspecified),
            "INCONCLUSIVE_REASON_PREFIX_UNCERTAINTY" => Some(Self::PrefixUncertainty),
            "INCONCLUSIVE_REASON_OBSERVATIONAL_UNCERTAINTY" => Some(Self::ObservationalUncertainty),
            "INCONCLUSIVE_REASON_EPISTEMIC_UNCERTAINTY" => Some(Self::EpistemicUncertainty),
            "INCONCLUSIVE_REASON_ANCHOR_SILENCE" => Some(Self::AnchorSilence),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum VerdictAuthority {
    Unspecified = 0,
    Provisional = 1,
    Authoritative = 2,
}
impl VerdictAuthority {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            VerdictAuthority::Unspecified => "VERDICT_AUTHORITY_UNSPECIFIED",
            VerdictAuthority::Provisional => "VERDICT_AUTHORITY_PROVISIONAL",
            VerdictAuthority::Authoritative => "VERDICT_AUTHORITY_AUTHORITATIVE",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "VERDICT_AUTHORITY_UNSPECIFIED" => Some(Self::Unspecified),
            "VERDICT_AUTHORITY_PROVISIONAL" => Some(Self::Provisional),
            "VERDICT_AUTHORITY_AUTHORITATIVE" => Some(Self::Authoritative),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum TierSettlement {
    Unspecified = 0,
    NotRequired = 1,
    Pending = 2,
    Settled = 3,
}
impl TierSettlement {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            TierSettlement::Unspecified => "TIER_SETTLEMENT_UNSPECIFIED",
            TierSettlement::NotRequired => "TIER_SETTLEMENT_NOT_REQUIRED",
            TierSettlement::Pending => "TIER_SETTLEMENT_PENDING",
            TierSettlement::Settled => "TIER_SETTLEMENT_SETTLED",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "TIER_SETTLEMENT_UNSPECIFIED" => Some(Self::Unspecified),
            "TIER_SETTLEMENT_NOT_REQUIRED" => Some(Self::NotRequired),
            "TIER_SETTLEMENT_PENDING" => Some(Self::Pending),
            "TIER_SETTLEMENT_SETTLED" => Some(Self::Settled),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum LateDataPolicy {
    Unspecified = 0,
    Exclude = 1,
    AcceptForFutureVerdicts = 2,
}
impl LateDataPolicy {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            LateDataPolicy::Unspecified => "LATE_DATA_POLICY_UNSPECIFIED",
            LateDataPolicy::Exclude => "LATE_DATA_POLICY_EXCLUDE",
            LateDataPolicy::AcceptForFutureVerdicts => "LATE_DATA_POLICY_ACCEPT_FOR_FUTURE_VERDICTS",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "LATE_DATA_POLICY_UNSPECIFIED" => Some(Self::Unspecified),
            "LATE_DATA_POLICY_EXCLUDE" => Some(Self::Exclude),
            "LATE_DATA_POLICY_ACCEPT_FOR_FUTURE_VERDICTS" => Some(Self::AcceptForFutureVerdicts),
            _ => None,
        }
    }
}
/// Whether this Verdict was computed while a Specification was warming behind
/// an activation barrier. Three-way, not a bool: NOT_DECLARED is distinct from
/// CLEARED so a consumer cannot read "cleared" as evidence that a rollout
/// completed when none was declared.
///
/// A warming Shadow Violate is attributable as Violate ∧ Provisional ∧ PENDING.
/// Comparing acknowledged Filter epochs against target_filter_epoch is not that
/// predicate: a barrier can stay pending after every present acknowledged epoch
/// has reached the target (watermark lag, no complete boundary, …).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, ::prost::Enumeration)]
#[repr(i32)]
pub enum ActivationBarrierStatus {
    Unspecified = 0,
    NotDeclared = 1,
    Pending = 2,
    Cleared = 3,
}
impl ActivationBarrierStatus {
    /// String value of the enum field names used in the ProtoBuf definition.
    ///
    /// The values are not transformed in any way and thus are considered stable
    /// (if the ProtoBuf definition does not change) and safe for programmatic use.
    pub fn as_str_name(&self) -> &'static str {
        match self {
            ActivationBarrierStatus::Unspecified => "ACTIVATION_BARRIER_STATUS_UNSPECIFIED",
            ActivationBarrierStatus::NotDeclared => "ACTIVATION_BARRIER_STATUS_NOT_DECLARED",
            ActivationBarrierStatus::Pending => "ACTIVATION_BARRIER_STATUS_PENDING",
            ActivationBarrierStatus::Cleared => "ACTIVATION_BARRIER_STATUS_CLEARED",
        }
    }
    /// Creates an enum from field names used in the ProtoBuf definition.
    pub fn from_str_name(value: &str) -> ::core::option::Option<Self> {
        match value {
            "ACTIVATION_BARRIER_STATUS_UNSPECIFIED" => Some(Self::Unspecified),
            "ACTIVATION_BARRIER_STATUS_NOT_DECLARED" => Some(Self::NotDeclared),
            "ACTIVATION_BARRIER_STATUS_PENDING" => Some(Self::Pending),
            "ACTIVATION_BARRIER_STATUS_CLEARED" => Some(Self::Cleared),
            _ => None,
        }
    }
}
// @@protoc_insertion_point(module)
