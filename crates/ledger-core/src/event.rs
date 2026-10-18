//! Ledger events the adapter turns into Sentinel events.

use std::collections::BTreeMap;

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

/// Exchange order identity. The response monitor binds one slice per order.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OrderId(String);

impl OrderId {
    /// Parses an order id.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Blank`] when `value` is empty or whitespace.
    pub fn new(value: impl Into<String>) -> Result<Self, AdapterError> {
        Ok(Self(non_blank(value.into(), "order id")?))
    }

    /// Returns the order id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How an order left the book. Both match the response selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrderEnd {
    /// The order filled.
    Fill,
    /// The order was cancelled.
    Cancel,
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
    /// A bill memo was read. The semantic panel is stored on this event.
    BillMemo,
    /// The agent recorded a decision rationale. Its panel is stored here.
    AgentDecision,
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
    /// An approval recorded for the bill.
    ///
    /// Precedence requires this in the payment's causal past. The record is an
    /// anchor source: the monitored payer cannot forge it, and silence of that
    /// source is what settles a missing approval.
    Approval,
    /// The exchange accepted an order. This opens the response scope.
    OrderSubmit,
    /// The exchange filled or cancelled an order.
    OrderResolved,
    /// The response window closed after [`crate::RESPONSE_DEADLINE_TICKS`] ticks.
    OrderDeadline,
    /// One exchange tick. Three of these close an open order's response window.
    Tick,
}

impl LedgerKind {
    /// Every kind, in documentation order.
    pub const ALL: [Self; 11] = [
        Self::AgentPayBill,
        Self::BillMemo,
        Self::AgentDecision,
        Self::ObligationSettled,
        Self::ExtraPayment,
        Self::Credit,
        Self::Approval,
        Self::OrderSubmit,
        Self::OrderResolved,
        Self::OrderDeadline,
        Self::Tick,
    ];

    /// Sentinel event kind for this ledger event.
    #[must_use]
    pub const fn event_kind(self) -> &'static str {
        match self {
            Self::AgentPayBill => "ledger.agent.pay_bill",
            Self::BillMemo => "ledger.agent.memo",
            Self::AgentDecision => "ledger.agent.decision",
            Self::ObligationSettled => "ledger.exchange.settled",
            Self::ExtraPayment => "ledger.exchange.payment",
            Self::Credit => "ledger.exchange.credit",
            Self::Approval => "ledger.exchange.approval",
            Self::OrderSubmit => "ledger.exchange.order_submit",
            Self::OrderResolved => "ledger.exchange.order_resolved",
            Self::OrderDeadline => "ledger.exchange.order_deadline",
            Self::Tick => "ledger.exchange.tick",
        }
    }

    /// Whether this kind is produced by the exchange rather than an agent log.
    #[must_use]
    pub const fn is_exchange(self) -> bool {
        match self {
            Self::AgentPayBill | Self::BillMemo | Self::AgentDecision => false,
            Self::ObligationSettled
            | Self::ExtraPayment
            | Self::Credit
            | Self::Approval
            | Self::OrderSubmit
            | Self::OrderResolved
            | Self::OrderDeadline
            | Self::Tick => true,
        }
    }

    /// Whether the event carries a bill id.
    #[must_use]
    pub const fn carries_bill(self) -> bool {
        match self {
            Self::AgentPayBill
            | Self::BillMemo
            | Self::AgentDecision
            | Self::ObligationSettled
            | Self::ExtraPayment
            | Self::Credit
            | Self::Approval => true,
            Self::OrderSubmit | Self::OrderResolved | Self::OrderDeadline | Self::Tick => false,
        }
    }

    /// Whether the event carries an order id.
    #[must_use]
    pub const fn carries_order(self) -> bool {
        match self {
            Self::OrderSubmit | Self::OrderResolved | Self::OrderDeadline => true,
            Self::AgentPayBill
            | Self::BillMemo
            | Self::AgentDecision
            | Self::ObligationSettled
            | Self::ExtraPayment
            | Self::Credit
            | Self::Approval
            | Self::Tick => false,
        }
    }

    /// Role name used on a counterexample timeline.
    #[must_use]
    pub const fn role(self) -> &'static str {
        match self {
            Self::AgentPayBill => "payment",
            Self::BillMemo => "memo",
            Self::AgentDecision => "decision",
            Self::ObligationSettled => "settlement",
            Self::ExtraPayment => "second-payment",
            Self::Credit => "credit",
            Self::Approval => "approval",
            Self::OrderSubmit => "order",
            Self::OrderResolved => "fill-or-cancel",
            Self::OrderDeadline => "deadline",
            Self::Tick => "tick",
        }
    }

    /// Parses a stored kind or the span name ingest keeps for unadapted rows.
    #[must_use]
    pub fn from_stored_name(name: &str) -> Option<Self> {
        if let Some(kind) = Self::ALL.into_iter().find(|kind| kind.event_kind() == name) {
            return Some(kind);
        }
        match name {
            "agent.pay_bill" => Some(Self::AgentPayBill),
            "agent.memo" => Some(Self::BillMemo),
            "agent.decision" => Some(Self::AgentDecision),
            "agent.approve" => Some(Self::Approval),
            "exchange.submit_order" => Some(Self::OrderSubmit),
            "exchange.fill" => Some(Self::OrderResolved),
            "exchange.cancel" => Some(Self::OrderResolved),
            "exchange.tick" => Some(Self::Tick),
            _ => None,
        }
    }
}

/// One simulated ledger event, before it is adapted into a Sentinel `Event`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerEvent {
    id: String,
    kind: LedgerKind,
    bill_id: Option<BillId>,
    order_id: Option<OrderId>,
    end: Option<OrderEnd>,
    agent_id: AgentId,
    times: Times,
    predecessors: Vec<String>,
    derived: bool,
    /// Typed attributes. Panel probabilities and the threshold flag live here.
    attributes: BTreeMap<String, String>,
}

impl LedgerEvent {
    /// Builds a bill event.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Blank`] when `id` or a predecessor id is empty,
    /// [`AdapterError::SelfPredecessor`] when the event names itself, and
    /// [`AdapterError::KindMismatch`] when `kind` does not carry a bill.
    pub fn new(
        id: impl Into<String>,
        kind: LedgerKind,
        bill_id: BillId,
        agent_id: AgentId,
        times: Times,
        predecessors: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, AdapterError> {
        if !kind.carries_bill() {
            return Err(AdapterError::KindMismatch {
                kind: kind.event_kind(),
                subject: "bill",
            });
        }
        Self::assemble(
            id,
            kind,
            Some(bill_id),
            None,
            None,
            agent_id,
            times,
            predecessors,
        )
    }

    /// Builds an order event.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Blank`] when `id` or a predecessor id is empty,
    /// [`AdapterError::SelfPredecessor`] when the event names itself, and
    /// [`AdapterError::KindMismatch`] when `kind` does not carry an order.
    pub fn order(
        id: impl Into<String>,
        kind: LedgerKind,
        order_id: OrderId,
        end: Option<OrderEnd>,
        agent_id: AgentId,
        times: Times,
        predecessors: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, AdapterError> {
        if !kind.carries_order() {
            return Err(AdapterError::KindMismatch {
                kind: kind.event_kind(),
                subject: "order",
            });
        }
        Self::assemble(
            id,
            kind,
            None,
            Some(order_id),
            end,
            agent_id,
            times,
            predecessors,
        )
    }

    /// Builds an exchange tick. It is not bound to a bill or an order.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Blank`] when `id` or a predecessor id is empty,
    /// and [`AdapterError::SelfPredecessor`] when the event names itself.
    pub fn tick(
        id: impl Into<String>,
        agent_id: AgentId,
        times: Times,
        predecessors: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, AdapterError> {
        Self::assemble(
            id,
            LedgerKind::Tick,
            None,
            None,
            None,
            agent_id,
            times,
            predecessors,
        )
    }

    fn assemble(
        id: impl Into<String>,
        kind: LedgerKind,
        bill_id: Option<BillId>,
        order_id: Option<OrderId>,
        end: Option<OrderEnd>,
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
            order_id,
            end,
            agent_id,
            times,
            predecessors: causal,
            derived: false,
            attributes: BTreeMap::new(),
        })
    }

    /// Stores `value` under `key` when both are non-blank.
    #[must_use]
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        let value = value.into();
        if !key.trim().is_empty() && !value.trim().is_empty() {
            self.attributes.insert(key, value);
        }
        self
    }

    /// Returns a stored attribute.
    #[must_use]
    pub fn attribute(&self, key: &str) -> Option<&str> {
        self.attributes.get(key).map(String::as_str)
    }

    /// Returns every stored attribute, in key order.
    #[must_use]
    pub fn attributes(&self) -> &BTreeMap<String, String> {
        &self.attributes
    }

    /// Changes the kind. The bill or order subject stays.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::KindMismatch`] when `kind` needs a subject this
    /// event does not have.
    pub fn recast(mut self, kind: LedgerKind) -> Result<Self, AdapterError> {
        if kind.carries_bill() && self.bill_id.is_none() {
            return Err(AdapterError::KindMismatch {
                kind: kind.event_kind(),
                subject: "bill",
            });
        }
        if kind.carries_order() && self.order_id.is_none() {
            return Err(AdapterError::KindMismatch {
                kind: kind.event_kind(),
                subject: "order",
            });
        }
        self.kind = kind;
        Ok(self)
    }

    /// Records that the engine derived this close from stored evidence.
    #[must_use]
    pub fn derived(mut self) -> Self {
        self.derived = true;
        self
    }

    /// Whether the engine derived this event from other stored evidence.
    #[must_use]
    pub const fn is_derived(&self) -> bool {
        self.derived
    }

    /// Adds `predecessor` when it is not already listed.
    ///
    /// # Errors
    ///
    /// Returns [`AdapterError::Blank`] when `predecessor` is empty, and
    /// [`AdapterError::SelfPredecessor`] when it names this event.
    pub fn precede_by(&mut self, predecessor: &str) -> Result<(), AdapterError> {
        let predecessor = non_blank(predecessor.to_owned(), "predecessor id")?;
        if predecessor == self.id {
            return Err(AdapterError::SelfPredecessor {
                event: self.id.clone(),
            });
        }
        if !self.predecessors.iter().any(|id| id == &predecessor) {
            self.predecessors.push(predecessor);
        }
        Ok(())
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
    pub const fn bill_id(&self) -> Option<&BillId> {
        self.bill_id.as_ref()
    }

    /// Returns the order this event applies to.
    #[must_use]
    pub const fn order_id(&self) -> Option<&OrderId> {
        self.order_id.as_ref()
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

    /// Timeline role. A fill or cancel replaces the shared resolved role.
    #[must_use]
    pub const fn role(&self) -> &'static str {
        match self.end {
            Some(OrderEnd::Fill) => "fill",
            Some(OrderEnd::Cancel) => "cancel",
            None => self.kind.role(),
        }
    }

    /// Source name stored beside the event: `exchange` or `agent.{id}`.
    #[must_use]
    pub fn source_name(&self) -> String {
        if self.kind.is_exchange() {
            "exchange".to_owned()
        } else {
            format!("agent.{}", self.agent_id.as_str())
        }
    }
}

pub(crate) fn non_blank(value: String, field: &'static str) -> Result<String, AdapterError> {
    if value.trim().is_empty() {
        Err(AdapterError::Blank(field))
    } else {
        Ok(value)
    }
}
