//! Ledger events to Sentinel `Event`s.
//!
//! Completeness signals follow the shape in Sentinel
//! `core/tests/absence_evaluator.rs`: one signal per source, `complete_through`
//! equal to the last emitted sequence, no gaps. That file is a test fixture in
//! the pre-existing Sentinel repo and is not copied here.

use std::collections::{BTreeMap, BTreeSet};

use sentinel_model::{
    AttributeKey, AttributeValue, ClockDomainId, Event, EventId, ObservationTime, OccurrenceTime,
    ProducerCompletenessSignal, ProducerEvent, SequenceNumber, SourceActivity,
    SourceCompletenessSignal, SourceEvidence, SourceIdentity, SourceTier,
};

use crate::error::AdapterError;
use crate::event::{AgentId, LedgerEvent};
use crate::source::{
    agent_source, assurance, capabilities, clock_domain, data_class, event_kind, exchange_source,
    filter_epoch, schema_version, sensitivity, sentinel_clock, sequence_epoch, tier_of,
    AGENT_ATTRIBUTE, BILL_ATTRIBUTE, ORDER_ATTRIBUTE,
};

/// Which required source declared no completeness signal.
///
/// Events from that source are still delivered. Missing evidence closes the
/// gate, so a verdict that would have been `Satisfy` stays `Inconclusive`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WithheldSource {
    /// The exchange declared nothing.
    Exchange,
    /// The judged agent's tool-call log declared nothing.
    AgentToolLog,
}

/// Sentinel events and the per-source evidence that accompanies them.
#[derive(Clone, Debug)]
pub struct AdaptedTrace {
    events: Vec<Event>,
    evidence: Vec<SourceEvidence>,
}

impl AdaptedTrace {
    /// Returns adapted events in trace order.
    #[must_use]
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Returns one completeness record per source that was not withheld.
    #[must_use]
    pub fn evidence(&self) -> &[SourceEvidence] {
        &self.evidence
    }

    pub(crate) fn into_parts(self) -> (Vec<Event>, Vec<SourceEvidence>) {
        (self.events, self.evidence)
    }
}

#[derive(Debug)]
struct Rollup {
    tier: SourceTier,
    count: u64,
    clock: String,
    earliest_occurred: i128,
    latest_occurred: i128,
    latest_observed: i128,
}

/// Incremental adapter. Each [`TraceCursor::push`] assigns the next per-source
/// sequence. [`TraceCursor::evidence`] closes those sequences for a seal clone.
/// The live cursor itself stays open so later events can still arrive.
#[derive(Debug, Default)]
pub struct TraceCursor {
    seen: BTreeSet<String>,
    rollups: BTreeMap<SourceIdentity, Rollup>,
}

impl TraceCursor {
    /// Creates an empty cursor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adapts one ledger event.
    ///
    /// # Errors
    ///
    /// Returns an error when an id is blank, a predecessor is unknown, a source
    /// sequence overflows, or a Sentinel constructor rejects the event.
    pub fn push(&mut self, event: &LedgerEvent) -> Result<Event, AdapterError> {
        let (source, tier) = source_of(event)?;
        let sequence = next_sequence(self.rollups.get(&source).map(|rollup| rollup.count))?;
        let clock = clock_domain(event.kind(), event.agent_id())?;
        let predecessors = predecessor_ids(event, &self.seen)?;
        if !self.seen.insert(event.id().to_owned()) {
            return Err(AdapterError::DuplicateEvent(event.id().to_owned()));
        }
        let adapted = sentinel_event(event, &source, tier, sequence, &clock, &predecessors)?;
        note_rollup(
            &mut self.rollups,
            source,
            tier,
            sequence,
            clock.as_str(),
            event,
        );
        Ok(adapted)
    }

    /// Completeness signals for every source this cursor has seen.
    ///
    /// Applying these to a clone seals that clone. The cursor stays open.
    ///
    /// # Errors
    ///
    /// Returns an error when a completeness signal is rejected.
    pub fn evidence(&self) -> Result<Vec<SourceEvidence>, AdapterError> {
        let mut evidence = Vec::with_capacity(self.rollups.len());
        for (source, rollup) in &self.rollups {
            evidence.push(completeness(source, rollup)?);
        }
        Ok(evidence)
    }
}

/// Adapts `events` for the monitor that watches `agent`'s tool-call log.
///
/// The exchange is one source. `agent`'s tool-call log is another. A second
/// agent's log is a third source; [`WithheldSource::AgentToolLog`] withholds
/// only the judged agent's log.
///
/// # Errors
///
/// Returns an error when an id is blank, a predecessor is unknown, a source
/// sequence overflows, or a Sentinel constructor rejects the event.
pub fn adapt_trace(
    agent: &AgentId,
    events: &[LedgerEvent],
    withheld: Option<WithheldSource>,
) -> Result<AdaptedTrace, AdapterError> {
    let mut cursor = TraceCursor::new();
    let mut adapted = Vec::with_capacity(events.len());
    for event in events {
        adapted.push(cursor.push(event)?);
    }
    let mut evidence = Vec::new();
    for item in cursor.evidence()? {
        if is_withheld(item.source(), agent, withheld)? {
            continue;
        }
        evidence.push(item);
    }
    Ok(AdaptedTrace {
        events: adapted,
        evidence,
    })
}

fn source_of(event: &LedgerEvent) -> Result<(SourceIdentity, SourceTier), AdapterError> {
    let tier = tier_of(event.kind());
    let source = if event.kind().is_exchange() {
        exchange_source()
    } else {
        agent_source(event.agent_id())?
    };
    Ok((source, tier))
}

fn next_sequence(previous: Option<u64>) -> Result<u64, AdapterError> {
    previous
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(AdapterError::SequenceOverflow)
}

fn predecessor_ids(
    event: &LedgerEvent,
    seen: &BTreeSet<String>,
) -> Result<BTreeSet<EventId>, AdapterError> {
    let mut predecessors = BTreeSet::new();
    for predecessor in event.predecessors() {
        if !seen.contains(predecessor) {
            return Err(AdapterError::UnknownPredecessor {
                event: event.id().to_owned(),
                predecessor: predecessor.clone(),
            });
        }
        predecessors.insert(EventId::new(predecessor.clone())?);
    }
    Ok(predecessors)
}

fn note_rollup(
    rollups: &mut BTreeMap<SourceIdentity, Rollup>,
    source: SourceIdentity,
    tier: SourceTier,
    sequence: u64,
    clock: &str,
    event: &LedgerEvent,
) {
    let occurred = event.times().occurred_at_ns;
    let observed = event.times().observed_at_ns;
    rollups
        .entry(source)
        .and_modify(|rollup| {
            rollup.count = sequence;
            rollup.earliest_occurred = rollup.earliest_occurred.min(occurred);
            rollup.latest_occurred = rollup.latest_occurred.max(occurred);
            rollup.latest_observed = rollup.latest_observed.max(observed);
        })
        .or_insert(Rollup {
            tier,
            count: sequence,
            clock: clock.to_owned(),
            earliest_occurred: occurred,
            latest_occurred: occurred,
            latest_observed: observed,
        });
}

fn sentinel_event(
    event: &LedgerEvent,
    source: &SourceIdentity,
    tier: SourceTier,
    sequence: u64,
    clock: &ClockDomainId,
    predecessors: &BTreeSet<EventId>,
) -> Result<Event, AdapterError> {
    let mut attributes = BTreeMap::from([(
        attribute_key(AGENT_ATTRIBUTE),
        AttributeValue::String(event.agent_id().as_str().to_owned()),
    )]);
    if let Some(bill) = event.bill_id() {
        attributes.insert(
            attribute_key(BILL_ATTRIBUTE),
            AttributeValue::String(bill.as_str().to_owned()),
        );
    }
    if let Some(order) = event.order_id() {
        attributes.insert(
            attribute_key(ORDER_ATTRIBUTE),
            AttributeValue::String(order.as_str().to_owned()),
        );
    }
    let producer = ProducerEvent::new(
        EventId::new(event.id())?,
        sequence_epoch(),
        SequenceNumber::new(sequence),
        schema_version(),
        filter_epoch(),
        event_kind(event.kind()),
        OccurrenceTime::new(clock.clone(), event.times().occurred_at_ns, 0),
        attributes,
        capabilities(),
        sensitivity(),
        predecessors.clone(),
    )?;
    let observed = ObservationTime::new(sentinel_clock(), event.times().observed_at_ns);
    Ok(Event::observe(
        producer,
        assurance(source.clone(), tier)?,
        observed,
        data_class(),
    ))
}

fn completeness(source: &SourceIdentity, rollup: &Rollup) -> Result<SourceEvidence, AdapterError> {
    let clock = ClockDomainId::new(rollup.clock.clone())?;
    let since = OccurrenceTime::new(clock.clone(), rollup.earliest_occurred, 0);
    let reported = OccurrenceTime::new(clock, rollup.latest_occurred, 0);
    let producer = ProducerCompletenessSignal::new(
        sequence_epoch(),
        Some(SequenceNumber::new(rollup.count)),
        Some(SequenceNumber::new(rollup.count)),
        reported,
        filter_epoch(),
        SourceActivity::Active { since },
    )?;
    let observed = ObservationTime::new(sentinel_clock(), rollup.latest_observed);
    Ok(SourceEvidence::new(
        source.clone(),
        rollup.tier,
        capabilities(),
        SourceCompletenessSignal::observe(
            producer,
            assurance(source.clone(), rollup.tier)?,
            observed,
        ),
        Vec::new(),
    ))
}

fn is_withheld(
    source: &SourceIdentity,
    agent: &AgentId,
    withheld: Option<WithheldSource>,
) -> Result<bool, AdapterError> {
    let Some(withheld) = withheld else {
        return Ok(false);
    };
    let withheld_source = match withheld {
        WithheldSource::Exchange => exchange_source(),
        WithheldSource::AgentToolLog => agent_source(agent)?,
    };
    Ok(source == &withheld_source)
}

fn attribute_key(name: &'static str) -> AttributeKey {
    AttributeKey::new(name).expect("constant attribute key is non-blank")
}
