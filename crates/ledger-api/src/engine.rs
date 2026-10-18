//! Poll the events hypertable and seal verdicts.
//!
//! Counts stay on [`ledger_store::Store::window_counts`]. This loop reads the
//! same `observed_at` column, one row at a time, and steps Sentinel.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use ledger_core::{Judged, LedgerEvent, LedgerKind, Session, TimelineEntry, VerdictOutcome};
use ledger_jev::{
    rank_exchange, rerank_request, strip_explanation, Candidate, Counterexample, JevCallFailure,
    Ranking, Relevance, SystemOne, ViolatedSpec,
};
use ledger_store::{Event as StoredEvent, Store, Uuid, Verdict, VerdictOutcome as StoredOutcome};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::str::FromStr;
use thiserror::Error;
use tokio::sync::broadcast;

use crate::decode::{self, read_event};

const PAGE: i64 = 200;
const VERDICT_NAMESPACE: Uuid = Uuid::from_u128(0x6c65_6467_6572_5f76_6572_6469_6374_0001);

/// Why a poll or a stream could not finish.
#[derive(Debug, Error)]
pub enum ApiError {
    /// Timescale rejected a query.
    #[error(transparent)]
    Store(#[from] ledger_store::StoreError),
    /// A ledger event could not be adapted.
    #[error(transparent)]
    Adapt(#[from] ledger_core::AdapterError),
    /// The HTTP listener could not bind.
    #[error("failed to bind the verdict API: {0}")]
    Bind(#[from] std::io::Error),
}

/// One SSE `data` object. `kind` is `verdict`. The verdict fields match the UI.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamMessage {
    /// Always `verdict`.
    pub kind: &'static str,
    /// The verdict the UI applies.
    pub verdict: StreamVerdict,
}

/// Verdict payload. `timeline` is the counterexample, built from evidence only.
/// Four raw Jev scores. A null dimension is Unknown, not a priority of zero.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StreamRank {
    /// Money at stake, on 0..=4.
    pub money: Option<f64>,
    /// Evidence strength, on 0..=4.
    pub evidence: Option<f64>,
    /// Binding ambiguity, on 0..=4.
    pub ambiguity: Option<f64>,
    /// Repeated action versus one-off, on 0..=4.
    pub intent: Option<f64>,
    /// System One request. No API key.
    pub input: serde_json::Value,
    /// System One response, or a failure marker. No explanation text.
    pub output: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StreamVerdict {
    /// Stable id for this spec, binding, and outcome.
    pub id: Uuid,
    /// `Satisfy`, `Violate`, or `Inconclusive`.
    pub outcome: String,
    /// `needs review` when the outcome is `Violate`.
    pub title: String,
    /// Timeline rendered as evidence ids and roles. No model prose.
    pub evidence: String,
    /// Specification id.
    #[serde(rename = "specId")]
    pub spec_id: String,
    /// Bill id or order id.
    pub binding: String,
    /// Stable reason code.
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<String>,
    /// Payment amount in cents, when the evidence carried one.
    #[serde(rename = "amountCents", skip_serializing_if = "Option::is_none")]
    pub amount_cents: Option<i64>,
    /// When the verdict was recorded.
    pub recorded_at: DateTime<Utc>,
    /// Evidence events in observation order.
    ///
    /// When `rank` is `ranked`, this is the minimal counterexample. When
    /// `rank` is `unknown`, this is the full trace.
    pub timeline: Vec<StreamEntry>,
    /// Exact, fuzzy, and Jev lines. Absent when this verdict is not a binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comparison: Option<StreamComparison>,
    /// Solana signatures on the counterexample, when the scene recorded them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signatures: Vec<String>,
    /// Events behind the expand control. Empty when the rerank failed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rest: Vec<StreamEntry>,
    /// Stored Jev input and mapped scores for the counterexample. No explanation text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rerank: Option<RerankRecord>,
    /// Stored raw scores. Absent until Jev has been asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<StreamRank>,
}

/// One exact, fuzzy, or Jev line on the comparison card.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamBindingLine {
    /// `Same`, `Different`, or `Unknown`.
    pub outcome: String,
    /// Stable reason code. No model prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

/// The three baselines stored with a binding verdict.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamComparison {
    /// Exact-match baseline.
    pub exact: StreamBindingLine,
    /// Fuzzy baseline.
    pub fuzzy: StreamBindingLine,
    /// Mapped Jev verdict.
    pub jev: StreamBindingLine,
}

/// One stored rerank. `failure` is a reason code, not model prose.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RerankRecord {
    /// Request body. It has no API key.
    pub request: String,
    /// Response body with explanation fields removed. Empty when the call failed.
    pub response: String,
    /// `timeout`, `contradiction`, `unusable`, and the other rank failures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// One mapped score per candidate, in trace order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scores: Vec<RerankScore>,
}

/// Mapped relevance for one event.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RerankScore {
    /// Ledger event id.
    #[serde(rename = "eventId")]
    pub event_id: String,
    /// `Relevant`, `NotRelevant`, or `Unknown`.
    pub outcome: String,
}

/// One step on the counterexample timeline.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StreamEntry {
    /// Stored event id, or a deadline id derived from tick evidence.
    pub event_id: String,
    /// Observation time.
    pub observed_at: String,
    /// `exchange` or `agent.{id}`.
    pub source: String,
    /// Event kind.
    pub kind: String,
    /// Role such as `payment` or `second-payment`.
    pub role: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredBody {
    binding: String,
    title: String,
    evidence: String,
    timeline: Vec<StreamEntry>,
    #[serde(default)]
    amount_cents: Option<i64>,
    #[serde(default)]
    comparison: Option<StreamComparison>,
    #[serde(default)]
    signatures: Vec<String>,
    #[serde(default)]
    scene: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    rest: Vec<StreamEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rerank: Option<RerankRecord>,
}

/// Reads events and publishes verdicts.
pub struct Engine {
    store: Store,
    session: Session,
    tx: broadcast::Sender<StreamVerdict>,
    seen: HashSet<Uuid>,
    dirty: bool,
    approvals: HashMap<String, String>,
    amounts: HashMap<String, i64>,
    emitted: HashSet<Uuid>,
    ranker: Option<Arc<dyn SystemOne>>,
    monitors: Option<crate::metrics::MonitorStats>,
}

impl Engine {
    /// Opens a session on `store` and publishes to `tx`.
    ///
    /// # Errors
    ///
    /// Returns an error when the response monitor cannot be instantiated.
    pub fn new(store: Store, tx: broadcast::Sender<StreamVerdict>) -> Result<Self, ApiError> {
        Self::with_ranker(store, tx, None)
    }

    /// Opens a session that scores verdicts and reranks violating traces.
    ///
    /// `None` leaves the evidence timeline unchanged and stores no scores.
    /// A rerank error shows the full trace in time order.
    ///
    /// # Errors
    ///
    /// Returns an error when the response monitor cannot be instantiated.
    pub fn with_ranker(
        store: Store,
        tx: broadcast::Sender<StreamVerdict>,
        ranker: Option<Arc<dyn SystemOne>>,
    ) -> Result<Self, ApiError> {
        Ok(Self {
            store,
            session: Session::new()?,
            tx,
            seen: HashSet::new(),
            dirty: false,
            approvals: HashMap::new(),
            amounts: HashMap::new(),
            emitted: HashSet::new(),
            ranker,
            monitors: None,
        })
    }

    /// Reports live and retired monitor counts to `stats` after each poll.
    pub(crate) fn report_monitors(&mut self, stats: crate::metrics::MonitorStats) {
        self.monitors = Some(stats);
    }

    /// Scores stored verdicts that have no rank row yet.
    ///
    /// A missing ranker does nothing. A Jev failure still stores Unknown
    /// dimensions so the inbox can fall back to severity order.
    ///
    /// # Errors
    ///
    /// Returns an error when a query fails.
    pub async fn score_unranked(&mut self) -> Result<(), ApiError> {
        if self.ranker.is_none() {
            return Ok(());
        }
        let verdicts = self.store.list_verdicts(1_000).await?;
        let existing = self.store.list_ranks().await?;
        let have: HashSet<Uuid> = existing.into_iter().map(|row| row.verdict_id).collect();
        for verdict in verdicts {
            if have.contains(&verdict.id) {
                continue;
            }
            let Some(frame) = verdict_from_stored(&verdict) else {
                continue;
            };
            let frame = self.attach_rank(frame).await;
            let _ = self.tx.send(frame);
        }
        Ok(())
    }

    /// Steps events observed in the last six hours, in `observed_at` order.
    ///
    /// The window is the same `observed_at` filter as [`Store::window_counts`].
    /// Events already stepped are left in the open monitors.
    ///
    /// # Errors
    ///
    /// Returns an error when a query fails or a seal fails for a reason other
    /// than missing evidence.
    pub async fn poll_once(&mut self) -> Result<Vec<StreamVerdict>, ApiError> {
        let now = Utc::now();
        let published = self
            .poll_window(now - Duration::hours(6), now + Duration::minutes(1))
            .await;
        if let Some(stats) = &self.monitors {
            stats.record(
                now,
                self.session.live_monitor_count(),
                self.session.retired_monitor_count(),
            );
        }
        published
    }

    /// Steps events in `[start, end)`, in `observed_at` order.
    ///
    /// Scenes pass a fixed clock here so a replay does not depend on wall time.
    ///
    /// # Errors
    ///
    /// Returns an error when a query fails or a seal fails for a reason other
    /// than missing evidence.
    pub async fn poll_window(
        &mut self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<StreamVerdict>, ApiError> {
        let mut published = Vec::new();
        let mut after = None;
        for _ in 0..50 {
            let page = match after {
                None => self.store.events_in_span(start, end, PAGE).await?,
                Some(cursor) => {
                    self.store
                        .events_in_span_after(start, end, cursor, PAGE)
                        .await?
                }
            };
            let count = page.len();
            if let Some(last) = page.last() {
                after = Some((last.observed_at, last.id));
            }
            published.extend(self.consume(page).await?);
            if count < usize::try_from(PAGE).unwrap_or(200) {
                break;
            }
        }
        Ok(published)
    }

    async fn consume(&mut self, rows: Vec<StoredEvent>) -> Result<Vec<StreamVerdict>, ApiError> {
        let mut published = Vec::new();
        for row in rows {
            if !self.seen.insert(row.id) {
                continue;
            }
            match read_event(&row) {
                Ok(None) => {}
                Ok(Some(mut event)) => {
                    self.remember(&row, &event);
                    self.attach_approval(&row, &mut event);
                    if let Some(cents) = amount_cents(&row) {
                        if let Some(bill) = event.bill_id() {
                            self.amounts.insert(bill.as_str().to_owned(), cents);
                        }
                    }
                    if let Err(error) = self.session.push(event) {
                        tracing::warn!(event_id = %row.id, error = %error, "event evidence failed");
                        published.push(self.failed(&row, "evidence-failed").await?);
                    } else {
                        self.dirty = true;
                    }
                }
                Err(reason) => {
                    tracing::warn!(event_id = %row.id, reason = %reason, "event evidence failed");
                    published.push(self.failed(&row, "evidence-failed").await?);
                }
            }
        }
        if self.dirty {
            published.extend(self.publish_session().await?);
            self.dirty = false;
        }
        Ok(published)
    }

    fn remember(&mut self, row: &StoredEvent, event: &LedgerEvent) {
        if event.kind() == LedgerKind::Approval {
            if let Some(approval) = decode::attribute(row, "approval.id") {
                self.approvals
                    .insert(approval.to_owned(), event.id().to_owned());
            }
        }
    }

    fn attach_approval(&self, row: &StoredEvent, event: &mut LedgerEvent) {
        if event.kind() != LedgerKind::AgentPayBill || !event.predecessors().is_empty() {
            return;
        }
        let Some(approval) = decode::attribute(row, "link.approval_id") else {
            return;
        };
        if let Some(id) = self.approvals.get(approval) {
            let _ = event.precede_by(id);
        }
    }

    async fn publish_session(&mut self) -> Result<Vec<StreamVerdict>, ApiError> {
        let judged = self.session.verdicts()?;
        let mut published = Vec::new();
        for item in judged {
            if let Some(frame) = self.store_judged(&item).await? {
                published.push(frame);
            }
        }
        Ok(published)
    }

    async fn store_judged(&mut self, item: &Judged) -> Result<Option<StreamVerdict>, ApiError> {
        let outcome = outcome_name(item.outcome);
        let reason = item.reason_code.clone();
        let id = verdict_id(&item.spec_id, &item.binding, outcome, reason.as_deref());
        if !self.emitted.insert(id) {
            return Ok(None);
        }
        let mut frame = stream_verdict(id, outcome, item, self.amounts.get(&item.binding).copied());
        if item.outcome == VerdictOutcome::Violate {
            if let Some(ranked) = self.rank_violation(item).await {
                frame.timeline = ranked.timeline;
                frame.rest = if ranked.counterexample == "full" {
                    Vec::new()
                } else {
                    ranked.rest
                };
                frame.rerank = Some(ranked.rerank);
                frame.evidence = evidence_of(&frame.timeline, item.reason_code.as_deref());
            }
        }
        let stored = Verdict {
            id,
            recorded_at: frame.recorded_at,
            outcome: stored_outcome(outcome),
            spec_id: item.spec_id.clone(),
            reason_code: reason,
            evidence: serde_json::to_value(stored_body(item, &frame))
                .unwrap_or_else(|_| serde_json::json!({})),
        };
        if self.store.insert_verdict_if_absent(&stored).await? {
            let frame = self.attach_rank(frame).await;
            let _ = self.tx.send(frame.clone());
            Ok(Some(frame))
        } else {
            Ok(None)
        }
    }

    async fn attach_rank(&self, mut frame: StreamVerdict) -> StreamVerdict {
        let Some(ranker) = self.ranker.clone() else {
            return frame;
        };
        let scored = crate::rank::score_frame(ranker, &frame).await;
        if let Err(error) = self
            .store
            .insert_rank_if_absent(&scored.row(frame.id, frame.recorded_at))
            .await
        {
            tracing::warn!(error = %error, verdict_id = %frame.id, "rank was not stored");
        }
        frame.rank = Some(scored.stream);
        frame
    }

    async fn failed(&mut self, row: &StoredEvent, reason: &str) -> Result<StreamVerdict, ApiError> {
        let binding = row.id.to_string();
        let id = verdict_id("ledger.evidence", &binding, "Inconclusive", Some(reason));
        let mut frame = StreamVerdict {
            id,
            outcome: "Inconclusive".to_owned(),
            title: "inconclusive".to_owned(),
            evidence: reason.to_owned(),
            spec_id: "ledger.evidence".to_owned(),
            binding: binding.clone(),
            reason_code: Some(reason.to_owned()),
            amount_cents: amount_cents(row),
            recorded_at: Utc::now(),
            timeline: vec![StreamEntry {
                event_id: binding,
                observed_at: row.observed_at.to_rfc3339(),
                source: row.source.clone(),
                kind: row.kind.clone(),
                role: "evidence".to_owned(),
            }],
            comparison: None,
            signatures: Vec::new(),
            rest: Vec::new(),
            rerank: None,
            rank: None,
        };
        if self.emitted.insert(id) {
            let stored = Verdict {
                id,
                recorded_at: frame.recorded_at,
                outcome: StoredOutcome::Inconclusive,
                spec_id: frame.spec_id.clone(),
                reason_code: frame.reason_code.clone(),
                evidence: serde_json::to_value(StoredBody {
                    binding: frame.binding.clone(),
                    title: frame.title.clone(),
                    evidence: frame.evidence.clone(),
                    timeline: frame.timeline.clone(),
                    amount_cents: frame.amount_cents,
                    comparison: None,
                    signatures: Vec::new(),
                    scene: None,
                    rest: Vec::new(),
                    rerank: None,
                })
                .unwrap_or_else(|_| serde_json::json!({})),
            };
            if self.store.insert_verdict_if_absent(&stored).await? {
                frame = self.attach_rank(frame).await;
                let _ = self.tx.send(frame.clone());
            }
        }
        Ok(frame)
    }
}

/// Rebuilds a stream verdict from a stored row. `None` when the evidence
/// document is not this engine's timeline.
#[must_use]
pub fn verdict_from_stored(verdict: &Verdict) -> Option<StreamVerdict> {
    let body: StoredBody = serde_json::from_value(verdict.evidence.clone()).ok()?;
    if body.timeline.is_empty() && body.evidence.is_empty() {
        return None;
    }
    Some(StreamVerdict {
        id: verdict.id,
        outcome: verdict.outcome.as_str().to_owned(),
        title: body.title,
        evidence: body.evidence,
        spec_id: verdict.spec_id.clone(),
        binding: body.binding,
        reason_code: verdict.reason_code.clone(),
        amount_cents: body.amount_cents,
        recorded_at: verdict.recorded_at,
        timeline: body.timeline,
        comparison: body.comparison,
        signatures: body.signatures,
        rest: body.rest,
        rerank: body.rerank,
        rank: None,
    })
}

async fn rank_call(ranker: Arc<dyn SystemOne>, request: String) -> Result<String, JevCallFailure> {
    match tokio::task::spawn_blocking(move || ranker.evaluate(&request)).await {
        Ok(result) => result,
        Err(_) => Err(JevCallFailure::Transport),
    }
}

fn stream_verdict(
    id: Uuid,
    outcome: &str,
    item: &Judged,
    amount_cents: Option<i64>,
) -> StreamVerdict {
    let timeline = item
        .timeline
        .iter()
        .map(|entry| StreamEntry {
            event_id: entry.event_id.clone(),
            observed_at: ns_to_rfc3339(entry.observed_at_ns),
            source: entry.source.clone(),
            kind: entry.kind.clone(),
            role: entry.role.clone(),
        })
        .collect::<Vec<_>>();
    let evidence = if timeline.is_empty() {
        item.reason_code
            .clone()
            .unwrap_or_else(|| "evidence".to_owned())
    } else {
        timeline
            .iter()
            .map(|entry| format!("{} {}", entry.role, entry.event_id))
            .collect::<Vec<_>>()
            .join(" → ")
    };
    StreamVerdict {
        id,
        outcome: outcome.to_owned(),
        title: title_for(outcome).to_owned(),
        evidence,
        spec_id: item.spec_id.clone(),
        binding: item.binding.clone(),
        reason_code: item.reason_code.clone(),
        amount_cents,
        recorded_at: Utc::now(),
        timeline,
        comparison: None,
        signatures: Vec::new(),
        rest: Vec::new(),
        rerank: None,
        rank: None,
    }
}

struct AppliedRank {
    timeline: Vec<StreamEntry>,
    rest: Vec<StreamEntry>,
    counterexample: &'static str,
    rerank: RerankRecord,
}

impl Engine {
    async fn rank_violation(&self, item: &Judged) -> Option<AppliedRank> {
        let ranker = Arc::clone(self.ranker.as_ref()?);
        let trace = self
            .session
            .trace(&item.spec_id, &item.binding, &item.matched);
        if trace.is_empty() {
            return None;
        }
        let candidates: Vec<Candidate> = trace.iter().map(candidate_of).collect();
        let spec = ViolatedSpec {
            id: &item.spec_id,
            reason: item.reason_code.as_deref().unwrap_or("needs-review"),
        };
        let request = rerank_request(spec, &item.matched, &candidates);
        let exchange = rank_call(ranker, request.clone()).await;
        let response = stored_response(&exchange);
        let ranking = match &exchange {
            Ok(body) => rank_exchange(&candidates, &item.matched, Ok(body.as_str())),
            Err(failure) => rank_exchange(&candidates, &item.matched, Err(*failure)),
        };
        Some(apply_counterexample(&trace, &ranking, request, response))
    }
}

fn candidate_of(entry: &TimelineEntry) -> Candidate {
    Candidate::new(
        entry.event_id.clone(),
        entry.role.clone(),
        entry.kind.clone(),
        entry.source.clone(),
    )
}

fn apply_counterexample(
    trace: &[TimelineEntry],
    ranking: &Ranking,
    request: String,
    response: String,
) -> AppliedRank {
    let entries = |ids: &[String]| -> Vec<StreamEntry> {
        ids.iter()
            .filter_map(|id| trace.iter().find(|entry| entry.event_id == *id))
            .map(stream_entry)
            .collect()
    };
    let (timeline, rest, counterexample) = match &ranking.counterexample {
        Counterexample::Selected { shown, rest } => (entries(shown), entries(rest), "ranked"),
        Counterexample::Full { events } => (entries(events), Vec::new(), "unknown"),
    };
    let timeline = if timeline.is_empty() {
        trace.iter().map(stream_entry).collect()
    } else {
        timeline
    };
    AppliedRank {
        timeline,
        rest,
        counterexample,
        rerank: RerankRecord {
            request,
            response,
            failure: ranking.failure.map(|failure| failure.code().to_owned()),
            scores: ranking
                .scores
                .iter()
                .map(|score| RerankScore {
                    event_id: score.id.clone(),
                    outcome: match score.relevance {
                        Relevance::Relevant => "Relevant",
                        Relevance::NotRelevant => "NotRelevant",
                        Relevance::Unknown => "Unknown",
                    }
                    .to_owned(),
                })
                .collect(),
        },
    }
}

fn stream_entry(entry: &TimelineEntry) -> StreamEntry {
    StreamEntry {
        event_id: entry.event_id.clone(),
        observed_at: ns_to_rfc3339(entry.observed_at_ns),
        source: entry.source.clone(),
        kind: entry.kind.clone(),
        role: entry.role.clone(),
    }
}

fn stored_body(item: &Judged, frame: &StreamVerdict) -> StoredBody {
    StoredBody {
        binding: item.binding.clone(),
        title: frame.title.clone(),
        evidence: frame.evidence.clone(),
        timeline: frame.timeline.clone(),
        amount_cents: frame.amount_cents,
        comparison: frame.comparison.clone(),
        signatures: frame.signatures.clone(),
        scene: None,
        rest: frame.rest.clone(),
        rerank: frame.rerank.clone(),
    }
}

fn stored_response(exchange: &Result<String, JevCallFailure>) -> String {
    let Ok(body) = exchange else {
        return String::new();
    };
    let Ok(mut value) = serde_json::from_str::<Value>(body) else {
        return String::new();
    };
    strip_explanation(&mut value);
    value.to_string()
}

fn evidence_of(timeline: &[StreamEntry], reason: Option<&str>) -> String {
    if timeline.is_empty() {
        return reason.unwrap_or("evidence").to_owned();
    }
    timeline
        .iter()
        .map(|entry| format!("{} {}", entry.role, entry.event_id))
        .collect::<Vec<_>>()
        .join(" → ")
}

fn title_for(outcome: &str) -> &'static str {
    match outcome {
        "Violate" => "needs review",
        "Satisfy" => "satisfied",
        _ => "inconclusive",
    }
}

fn outcome_name(outcome: VerdictOutcome) -> &'static str {
    match outcome {
        VerdictOutcome::Satisfy => "Satisfy",
        VerdictOutcome::Violate => "Violate",
        VerdictOutcome::Inconclusive | _ => "Inconclusive",
    }
}

fn stored_outcome(outcome: &str) -> StoredOutcome {
    match outcome {
        "Satisfy" => StoredOutcome::Satisfy,
        "Violate" => StoredOutcome::Violate,
        _ => StoredOutcome::Inconclusive,
    }
}

fn verdict_id(spec: &str, binding: &str, outcome: &str, reason: Option<&str>) -> Uuid {
    let key = format!("{spec}|{binding}|{outcome}|{}", reason.unwrap_or(""));
    Uuid::new_v5(&VERDICT_NAMESPACE, key.as_bytes())
}

fn ns_to_rfc3339(ns: i128) -> String {
    let seconds = i64::try_from(ns.div_euclid(1_000_000_000)).unwrap_or(0);
    let nanos = u32::try_from(ns.rem_euclid(1_000_000_000)).unwrap_or(0);
    DateTime::from_timestamp(seconds, nanos)
        .map(|time| time.to_rfc3339())
        .unwrap_or_else(|| ns.to_string())
}

fn amount_cents(row: &StoredEvent) -> Option<i64> {
    let text = decode::attribute(row, "amount")?;
    let decimal = Decimal::from_str(text).ok()?;
    let cents = decimal * Decimal::from(100);
    i64::try_from(cents).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_jev::{double_pay_candidates, double_pay_matched, rank_body};

    fn ids(entries: &[StreamEntry]) -> Vec<&str> {
        entries
            .iter()
            .map(|entry| entry.event_id.as_str())
            .collect()
    }

    fn trace_from_candidates() -> Vec<TimelineEntry> {
        double_pay_candidates()
            .into_iter()
            .enumerate()
            .map(|(index, event)| TimelineEntry {
                event_id: event.id,
                observed_at_ns: i128::from(u32::try_from(index).unwrap_or(0)) * 1_000_000_000,
                source: event.source,
                kind: event.kind,
                role: event.role,
            })
            .collect()
    }

    #[test]
    fn fixture_ranking_shows_the_three_key_events() {
        let body = include_str!("../../../fixtures/jev/rerank-double-pay.json");
        let ranking = rank_body(&double_pay_candidates(), &double_pay_matched(), body);
        let applied = apply_counterexample(
            &trace_from_candidates(),
            &ranking,
            "{}".to_owned(),
            body.to_owned(),
        );
        assert_eq!(applied.counterexample, "ranked");
        assert_eq!(ids(&applied.timeline), vec!["pay", "again", "again:close"]);
        assert_eq!(ids(&applied.rest), vec!["tick-1", "settled", "order-9"]);
        assert!(applied.rerank.failure.is_none());
        assert!(applied
            .timeline
            .windows(2)
            .all(|pair| pair[0].observed_at <= pair[1].observed_at));
        assert!(!body.to_ascii_lowercase().contains("explain"));
        assert!(
            !format!("{} {}", applied.rerank.request, applied.rerank.response)
                .to_ascii_lowercase()
                .contains("fraud")
        );
    }

    #[test]
    fn call_failure_shows_the_full_trace_in_time_order() {
        let ranking = rank_exchange(
            &double_pay_candidates(),
            &double_pay_matched(),
            Err(JevCallFailure::Timeout),
        );
        let applied = apply_counterexample(
            &trace_from_candidates(),
            &ranking,
            "{}".to_owned(),
            String::new(),
        );
        assert_eq!(applied.counterexample, "unknown");
        assert!(applied.rest.is_empty());
        assert_eq!(
            ids(&applied.timeline),
            vec![
                "pay",
                "tick-1",
                "settled",
                "order-9",
                "again",
                "again:close"
            ]
        );
        assert_eq!(applied.rerank.failure.as_deref(), Some("timeout"));
    }
}

/// JSON object written on the SSE stream.
#[must_use]
pub fn message(verdict: &StreamVerdict) -> StreamMessage {
    StreamMessage {
        kind: "verdict",
        verdict: verdict.clone(),
    }
}
