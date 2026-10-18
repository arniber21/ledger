//! Ledger events the adapter turns into Sentinel events.

use crate::error::AdapterError;

/// Company-scoped bill identity. The absence monitor binds one slice per bill.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BillId(String);

impl BillId {
    /// Parses a bill id.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Blank`] when `value` is empty or whitespace.
    pub fn new(value: impl Into<String>) -> Result<Self, AdapterError> {
        Ok(Self(non_blank(value.into(), "bill id")?))
    }

    /// Returns the bill id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Identity of one simulated buying agent.
///
/// Each agent has its own tool-call log, and that log is its own Sentinel source.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AgentId(String);

impl AgentId {
    /// Parses an agent id.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Blank`] when `value` is empty or whitespace.
    pub fn new(value: impl Into<String>) -> Result<Self, AdapterError> {
        Ok(Self(non_blank(value.into(), "agent id")?))
    }

    /// Returns the agent id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// When a ledger event occurred, and when this adapter observed it.
///
/// Occurrence time is the producer's clock. Observation time is the Sentinel
/// clock. The evaluator reads neither clock; both values arrive as data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Times {
    /// Producer-clock nanoseconds.
    pub occurred_at_ns: i128,
    /// Sentinel-clock nanoseconds at observation.
    pub observed_at_ns: i128,
}

impl Times {
    /// Uses one nanosecond count for both clocks.
    #[must_use]
    pub const fn at(nanoseconds: i128) -> Self {
        Self {
            occurred_at_ns: nanoseconds,
            observed_at_ns: nanoseconds,
        }
    }

    /// Names the two clocks separately.
    #[must_use]
    pub const fn new(occurred_at_ns: i128, observed_at_ns: i128) -> Self {
        Self {
            occurred_at_ns,
            observed_at_ns,
        }
    }
}

/// What happened, which also selects the Sentinel source and event kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerKind {
    /// The agent called `pay_bill`. Recorded on that agent's tool-call log.
    ///
    /// This opens the per-bill absence interval.
    AgentPayBill,
    /// The exchange settled the bill with its first payment.
    ///
    /// Not a second payment, so it is not the veto.
    ObligationSettled,
    /// The exchange posted another payment against an already-settled bill.
    ///
    /// This is the veto: a second payment inside the open interval.
    ExtraPayment,
    /// The exchange posted a credit, which closes the absence interval.
    ///
    /// A second payment already inside `(pay, credit]` still violates. A
    /// payment after the credit is outside that interval.
    Credit,
}

impl LedgerKind {
    /// Sentinel event kind for this ledger event.
    #[must_use]
    pub const fn event_kind(self) -> &'static str {
        match self {
            Self::AgentPayBill => "ledger.agent.pay_bill",
            Self::ObligationSettled => "ledger.exchange.settled",
            Self::ExtraPayment => "ledger.exchange.payment",
            Self::Credit => "ledger.exchange.credit",
        }
    }

    /// Whether this kind is produced by the exchange rather than an agent log.
    #[must_use]
    pub const fn is_exchange(self) -> bool {
        match self {
            Self::AgentPayBill => false,
            Self::ObligationSettled | Self::ExtraPayment | Self::Credit => true,
        }
    }
}

/// One simulated ledger event, before it is adapted into a Sentinel `Event`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerEvent {
    id: String,
    kind: LedgerKind,
    bill_id: BillId,
    agent_id: AgentId,
    times: Times,
    predecessors: Vec<String>,
}

impl LedgerEvent {
    /// Builds a ledger event.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Blank`] when `id` or a predecessor id is empty,
    /// and [`AdapterError::SelfPredecessor`] when the event names itself.
    pub fn new(
        id: impl Into<String>,
        kind: LedgerKind,
        bill_id: BillId,
        agent_id: AgentId,
        times: Times,
        predecessors: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, AdapterError> {
        let id = non_blank(id.into(), "event id")?;
        let mut causal = Vec::new();
        for predecessor in predecessors {
            let predecessor = non_blank(predecessor.into(), "predecessor id")?;
            if predecessor == id {
                return Err(AdapterError::SelfPredecessor { event: id });
            }
            causal.push(predecessor);
        }
        Ok(Self {
            id,
            kind,
            bill_id,
            agent_id,
            times,
            predecessors: causal,
        })
    }

    /// Returns the ledger event id. It becomes the Sentinel event id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the kind.
    #[must_use]
    pub const fn kind(&self) -> LedgerKind {
        self.kind
    }

    /// Returns the bill this event applies to.
    #[must_use]
    pub const fn bill_id(&self) -> &BillId {
        &self.bill_id
    }

    /// Returns the agent that acted.
    #[must_use]
    pub const fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    /// Returns occurrence and observation times.
    #[must_use]
    pub const fn times(&self) -> Times {
        self.times
    }

    /// Returns causal predecessor ids, in the order they were given.
    #[must_use]
    pub fn predecessors(&self) -> &[String] {
        &self.predecessors
    }
}

pub(crate) fn non_blank(value: String, field: &'static str) -> Result<String, AdapterError> {
    if value.trim().is_empty() {
        Err(AdapterError::Blank(field))
    } else {
        Ok(value)
    }
}
