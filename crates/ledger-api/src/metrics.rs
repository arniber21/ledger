//! Dashboard samples read from the store and the running monitor desk.
//!
//! `GET /api/metrics` returns counters and 10-minute series. Every number is
//! a count of stored rows or of live monitors. A source with no rows reads 0.
//! Monitor history starts when this process starts; it is not back-filled.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use ledger_store::{AttentionBucket, MetricTotals, RateBucket, Store};
use serde_json::{json, Value};

const RATE_SECONDS: i32 = 30;
const HISTORY: usize = 240;

/// One reading of the monitor desk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonitorSample {
    /// Unix milliseconds.
    pub t: i64,
    /// Monitors still watching.
    pub live: u64,
    /// Monitors retired by garbage collection.
    pub retired: u64,
}

/// Monitor counts shared between the verdict engine and the HTTP routes.
#[derive(Clone, Default)]
pub struct MonitorStats {
    samples: Arc<Mutex<VecDeque<MonitorSample>>>,
}

impl MonitorStats {
    /// Records the desk counts at `now`. An unchanged reading within ten
    /// seconds of the last one only moves that sample's time.
    pub fn record(&self, now: DateTime<Utc>, live: usize, retired: usize) {
        let sample = MonitorSample {
            t: now.timestamp_millis(),
            live: u64::try_from(live).unwrap_or(u64::MAX),
            retired: u64::try_from(retired).unwrap_or(u64::MAX),
        };
        let Ok(mut samples) = self.samples.lock() else {
            return;
        };
        if let Some(last) = samples.back_mut() {
            if sample.t - last.t < 10_000
                && (last.live, last.retired) == (sample.live, sample.retired)
            {
                last.t = sample.t;
                return;
            }
        }
        samples.push_back(sample);
        while samples.len() > HISTORY {
            samples.pop_front();
        }
    }

    fn since(&self, cutoff_ms: i64) -> Vec<MonitorSample> {
        self.samples
            .lock()
            .map(|samples| {
                samples
                    .iter()
                    .filter(|sample| sample.t >= cutoff_ms)
                    .copied()
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Clone)]
struct AppState {
    store: Store,
    monitors: MonitorStats,
}

/// Route for dashboard samples.
pub fn router(store: Store, monitors: MonitorStats) -> Router {
    Router::new()
        .route("/api/metrics", get(metrics))
        .with_state(AppState { store, monitors })
}

async fn metrics(State(state): State<AppState>) -> Result<Json<Value>, StatusCode> {
    let now = Utc::now();
    let start = now - Duration::minutes(10);
    let end = now + Duration::seconds(1);
    let fail = |error: ledger_store::StoreError| {
        tracing::warn!(error = %error, "metrics read failed");
        StatusCode::SERVICE_UNAVAILABLE
    };
    let totals = state.store.metric_totals(now).await.map_err(fail)?;
    let rates = state
        .store
        .rate_buckets(start, end, RATE_SECONDS)
        .await
        .map_err(fail)?;
    let attention = state
        .store
        .attention_buckets(start, end)
        .await
        .map_err(fail)?;
    let monitors = state.monitors.since(start.timestamp_millis());
    let rates = fill_rates(start, now, rates);
    let attention = fill_attention(start, now, attention);
    Ok(Json(body(&totals, &rates, &attention, &monitors)))
}

/// Puts a zero bucket wherever the store had no rows, so a quiet half minute
/// is a point on the line rather than a gap the chart joins across.
fn fill_rates(start: DateTime<Utc>, end: DateTime<Utc>, rows: Vec<RateBucket>) -> Vec<RateBucket> {
    let step = Duration::seconds(i64::from(RATE_SECONDS));
    let mut cursor = bucket_floor(start, i64::from(RATE_SECONDS));
    let mut filled = Vec::new();
    while cursor <= end {
        let row = rows.iter().find(|row| row.bucket == cursor);
        filled.push(row.cloned().unwrap_or(RateBucket {
            bucket: cursor,
            agent: 0,
            exchange: 0,
            solana: 0,
        }));
        cursor += step;
    }
    filled
}

fn fill_attention(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    rows: Vec<AttentionBucket>,
) -> Vec<AttentionBucket> {
    let mut cursor = bucket_floor(start, 60);
    let mut filled = Vec::new();
    while cursor <= end {
        let row = rows.iter().find(|row| row.bucket == cursor);
        filled.push(row.cloned().unwrap_or(AttentionBucket {
            bucket: cursor,
            violate: 0,
            inconclusive: 0,
        }));
        cursor += Duration::minutes(1);
    }
    filled
}

fn bucket_floor(time: DateTime<Utc>, seconds: i64) -> DateTime<Utc> {
    let floored = time.timestamp().div_euclid(seconds) * seconds;
    DateTime::from_timestamp(floored, 0).unwrap_or(time)
}

#[allow(clippy::cast_precision_loss)]
fn body(
    totals: &MetricTotals,
    rates: &[RateBucket],
    attention: &[AttentionBucket],
    monitors: &[MonitorSample],
) -> Value {
    let width = f64::from(RATE_SECONDS);
    let per_second = |count: i64| count as f64 / width;
    let sources = [
        rates.iter().any(|rate| rate.agent > 0),
        rates.iter().any(|rate| rate.exchange > 0),
        rates.iter().any(|rate| rate.solana > 0),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    let last = monitors.last();
    json!({
        "counters": {
            "eventsIngested": totals.events,
            "eventsPerSecond": totals.events_last_minute as f64 / 60.0,
            "sourceCount": sources,
            "liveMonitors": last.map_or(0, |sample| sample.live),
            "retiredByGc": last.map_or(0, |sample| sample.retired),
            "settledOnSolana": totals.settled_on_chain,
            "jevJudgments": totals.jev_judgments,
            "jevUnknown": totals.jev_unknown,
        },
        "rates": rates.iter().map(|rate| json!({
            "t": rate.bucket.timestamp_millis(),
            "agentLog": per_second(rate.agent),
            "exchange": per_second(rate.exchange),
            "solana": per_second(rate.solana),
        })).collect::<Vec<_>>(),
        "attention": attention.iter().map(|point| json!({
            "t": point.bucket.timestamp_millis(),
            "violate": point.violate,
            "inconclusive": point.inconclusive,
        })).collect::<Vec<_>>(),
        "monitors": monitors.iter().map(|sample| json!({
            "t": sample.t,
            "withGc": sample.live,
            "withoutGc": sample.live + sample.retired,
        })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_inputs_read_as_zero_not_as_data() {
        let value = body(&MetricTotals::default(), &[], &[], &[]);
        assert_eq!(value["counters"]["eventsIngested"], 0);
        assert_eq!(value["counters"]["liveMonitors"], 0);
        assert_eq!(value["rates"], json!([]));
        assert_eq!(value["monitors"], json!([]));
    }

    #[test]
    fn quiet_buckets_are_filled_with_zero() {
        let start = DateTime::from_timestamp(1_000_000_020, 0).expect("time");
        let end = start + Duration::seconds(95);
        let rows = vec![RateBucket {
            bucket: bucket_floor(start, 30) + Duration::seconds(30),
            agent: 3,
            exchange: 0,
            solana: 0,
        }];
        let filled = fill_rates(start, end, rows);
        assert_eq!(filled.len(), 4);
        assert_eq!(filled.iter().map(|row| row.agent).sum::<i64>(), 3);
        assert_eq!(filled[0].agent, 0);
    }

    #[test]
    fn monitors_without_gc_add_back_retired() {
        let stats = MonitorStats::default();
        stats.record(Utc::now(), 3, 2);
        let value = body(&MetricTotals::default(), &[], &[], &stats.since(0));
        assert_eq!(value["monitors"][0]["withGc"], 3);
        assert_eq!(value["monitors"][0]["withoutGc"], 5);
        assert_eq!(value["counters"]["retiredByGc"], 2);
    }
}
