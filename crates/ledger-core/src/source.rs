//! Sentinel source identities for the exchange and each agent's tool-call log.
//!
//! Tiers follow ADR-0022, filled at the adapter rather than defaulted:
//! the exchange is [`SourceTier::Anchor`] because the monitored agents cannot
//! suppress or forge its settlement record, and each agent's tool-call log is
//! [`SourceTier::Contributing`] because that log is inside the monitored
//! application. Events are [`DataClass::Tenant`]: they are the monitored
//! company's own activity, not donated corpus.

use sentinel_model::{
    ClockDomainId, DataClass, EventKind, FilterEpoch, Integrity, MonitoredApplicationId,
    SchemaVersion, Sensitivity, SentinelClockId, SequenceEpoch, SourceAssurance,
    SourceCapabilities, SourceCapability, SourceId, SourceIdentity, SourceTier,
};

use crate::error::AdapterError;
use crate::event::{AgentId, LedgerKind};

/// Monitored application that owns the exchange and the buying agents.
pub(crate) const APPLICATION: &str = "ledger";
/// Probe id of the exchange source.
pub(crate) const EXCHANGE_SOURCE: &str = "exchange";
/// Producer schema ledger Patterns are written against.
pub(crate) const SCHEMA_VERSION: &str = "ledger-events-v1";
/// Deployment topology stamped on receiver-owned assurance.
pub(crate) const TOPOLOGY_ID: &str = "ledger-sim";
/// Sentinel observation clock.
pub(crate) const SENTINEL_CLOCK: &str = "ledger.sentinel";
/// Exchange producer clock.
pub(crate) const EXCHANGE_CLOCK: &str = "ledger.exchange";
/// Attribute whose value is the bill id. One Binding per bill.
pub const BILL_ATTRIBUTE: &str = "ledger.bill.id";
/// Attribute whose value is the order id. One Binding per order.
pub const ORDER_ATTRIBUTE: &str = "ledger.order.id";
/// Attribute whose value is the acting agent.
pub const AGENT_ATTRIBUTE: &str = "ledger.agent.id";
/// Attribute set in code from the memo panel's payment question.
///
/// `yes` means `P(yes)` cleared 0.80. `no` means `P(no)` cleared 0.80.
/// `unknown` is a timeout, error, contradiction, or a score under both.
pub const TAINT_INSTRUCTS_ATTRIBUTE: &str = "ledger.taint.instructs_payment";
/// The memo panel said the text instructs a payment.
pub const TAINT_YES: &str = "yes";
/// The memo panel said the text does not instruct a payment.
pub const TAINT_NO: &str = "no";
/// The memo panel did not decide. Not a pass.
pub const TAINT_UNKNOWN: &str = "unknown";

/// Jsonb attribute key for one panel probability.
#[must_use]
pub fn taint_probability_key(question_id: &str) -> String {
    format!("ledger.taint.p.{question_id}")
}
/// Sequence epoch for a trace that does not restart numbering.
pub(crate) const SEQUENCE_EPOCH: u64 = 1;
/// Filter epoch acknowledged by every adapted event.
pub(crate) const FILTER_EPOCH: u64 = 1;

pub(crate) fn monitored_application() -> MonitoredApplicationId {
    MonitoredApplicationId::new(APPLICATION).expect("constant application id is non-blank")
}

/// Returns the producer schema version authored Patterns bind against.
#[must_use]
pub fn schema_version() -> SchemaVersion {
    SchemaVersion::new(SCHEMA_VERSION).expect("constant schema version is non-blank")
}

pub(crate) fn sentinel_clock() -> SentinelClockId {
    SentinelClockId::new(SENTINEL_CLOCK).expect("constant sentinel clock id is non-blank")
}

pub(crate) fn sequence_epoch() -> SequenceEpoch {
    SequenceEpoch::new(SEQUENCE_EPOCH)
}

pub(crate) fn filter_epoch() -> FilterEpoch {
    FilterEpoch::new(FILTER_EPOCH)
}

/// Returns the Sentinel event kind for `kind`.
#[must_use]
pub fn event_kind(kind: LedgerKind) -> EventKind {
    EventKind::new(kind.event_kind()).expect("constant event kind is non-blank")
}

/// Exchange settlement record. One source, shared by every agent.
#[must_use]
pub fn exchange_source() -> SourceIdentity {
    SourceIdentity::new(
        monitored_application(),
        SourceId::new(EXCHANGE_SOURCE).expect("constant exchange source id is non-blank"),
    )
}

/// One agent's tool-call log. Distinct from every other agent and from the exchange.
///
/// # Errors
///
/// Returns [`AdapterError::Identifier`] when the derived source id is rejected.
pub fn agent_source(agent: &AgentId) -> Result<SourceIdentity, AdapterError> {
    let source_id = format!("agent.{}", agent.as_str());
    Ok(SourceIdentity::new(
        monitored_application(),
        SourceId::new(source_id)?,
    ))
}

/// Clock domain of the producer that emits `kind` for `agent`.
///
/// # Errors
///
/// Returns [`AdapterError::Identifier`] when a derived agent clock id is rejected.
pub(crate) fn clock_domain(
    kind: LedgerKind,
    agent: &AgentId,
) -> Result<ClockDomainId, AdapterError> {
    let domain = if kind.is_exchange() {
        EXCHANGE_CLOCK.to_owned()
    } else {
        format!("ledger.agent.{}", agent.as_str())
    };
    Ok(ClockDomainId::new(domain)?)
}

/// Assurance tier of the source that emits `kind`.
#[must_use]
pub const fn tier_of(kind: LedgerKind) -> SourceTier {
    if kind.is_exchange() {
        SourceTier::Anchor
    } else {
        SourceTier::Contributing
    }
}

/// Capabilities both sources actually provide: causal edges and completeness.
#[must_use]
pub fn capabilities() -> SourceCapabilities {
    SourceCapabilities::new([
        SourceCapability::CausalEdges,
        SourceCapability::CompletenessSignals,
    ])
}

pub(crate) fn assurance(
    source: SourceIdentity,
    tier: SourceTier,
) -> Result<SourceAssurance, AdapterError> {
    Ok(SourceAssurance::new(
        source,
        tier,
        capabilities(),
        Integrity::Authenticated,
        Vec::new(),
        Vec::new(),
        TOPOLOGY_ID,
    )?)
}

/// Data class of every adapted ledger event.
#[must_use]
pub(crate) const fn data_class() -> DataClass {
    DataClass::Tenant
}

/// Sensitivity claim of simulated company books.
#[must_use]
pub(crate) const fn sensitivity() -> Sensitivity {
    Sensitivity::Internal
}
