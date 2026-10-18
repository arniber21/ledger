//! TimescaleDB storage for live events and verdicts.
//!
//! Events live in a hypertable partitioned on `observed_at`. Verdicts are a
//! regular table. An outcome is `Satisfy`, `Violate`, or `Inconclusive`.

use std::str::FromStr;
use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use thiserror::Error;

pub use chrono::{DateTime, Utc};
pub use uuid::Uuid;

/// Connection string for the local `docker-compose.yml` database.
///
/// The password is the local-development password from that file. It is not a
/// deployed credential. Set `DATABASE_URL` to override it.
pub const LOCAL_DATABASE_URL: &str = "postgres://ledger:ledger_local@127.0.0.1:5433/ledger";

/// Resolves the database URL.
///
/// Uses `DATABASE_URL` when it is set. Otherwise returns [`LOCAL_DATABASE_URL`].
#[must_use]
pub fn database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| LOCAL_DATABASE_URL.to_owned())
}

/// Failure from a store connection, migration, or row decode.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The database rejected a connection or a query.
    #[error("database error: {source}")]
    Database {
        #[from]
        source: sqlx::Error,
    },

    /// Applying the embedded migrations failed.
    #[error("migration failed: {source}")]
    Migrate {
        #[from]
        source: sqlx::migrate::MigrateError,
    },

    /// A stored outcome was not `Satisfy`, `Violate`, or `Inconclusive`.
    #[error("invalid verdict outcome: {0}")]
    InvalidOutcome(String),
}

/// `Satisfy`, `Violate`, or `Inconclusive`.
///
/// Missing evidence is `Inconclusive`. There is no pass outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictOutcome {
    /// The spec held for the evidence that was present.
    Satisfy,
    /// The spec was violated.
    Violate,
    /// Evidence was missing or failed, so the spec was not passed.
    Inconclusive,
}

impl VerdictOutcome {
    /// Returns the label stored in `verdicts.outcome`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Satisfy => "Satisfy",
            Self::Violate => "Violate",
            Self::Inconclusive => "Inconclusive",
        }
    }
}

impl TryFrom<&str> for VerdictOutcome {
    type Error = StoreError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "Satisfy" => Ok(Self::Satisfy),
            "Violate" => Ok(Self::Violate),
            "Inconclusive" => Ok(Self::Inconclusive),
            other => Err(StoreError::InvalidOutcome(other.to_owned())),
        }
    }
}

/// One stored ledger event.
///
/// Rows live in the `events` hypertable, partitioned on `observed_at`.
/// `payload` holds attributes later issues can extend.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Event {
    pub id: Uuid,
    pub observed_at: DateTime<Utc>,
    /// Source name, such as `exchange` or an agent id.
    pub source: String,
    /// Event kind, such as `bill.pay` or `order.submit`.
    pub kind: String,
    /// Sentinel data-class label. Stored so readers do not guess it.
    pub data_class: String,
    pub payload: serde_json::Value,
}

/// One stored verdict.
///
/// `evidence` is matched evidence only. The model writes no explanation text.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub id: Uuid,
    pub recorded_at: DateTime<Utc>,
    pub outcome: VerdictOutcome,
    pub spec_id: String,
    pub reason_code: Option<String>,
    pub evidence: serde_json::Value,
}

#[derive(sqlx::FromRow)]
struct VerdictRow {
    id: Uuid,
    recorded_at: DateTime<Utc>,
    outcome: String,
    spec_id: String,
    reason_code: Option<String>,
    evidence: serde_json::Value,
}

impl TryFrom<VerdictRow> for Verdict {
    type Error = StoreError;

    fn try_from(row: VerdictRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            recorded_at: row.recorded_at,
            outcome: VerdictOutcome::try_from(row.outcome.as_str())?,
            spec_id: row.spec_id,
            reason_code: row.reason_code,
            evidence: row.evidence,
        })
    }
}

/// Width passed to Timescale `time_bucket`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BucketWidth {
    /// One-minute buckets. This is the demo window.
    Minute,
    /// One-hour buckets.
    Hour,
}

/// One `time_bucket` count of stored events.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct WindowCount {
    /// Start of the bucket, inclusive.
    pub bucket: DateTime<Utc>,
    /// Event source, such as `exchange` or `agent.{id}`.
    pub source: String,
    /// Event kind, such as `agent.decide` or `ledger.agent.pay_bill`.
    pub kind: String,
    /// Rows in this bucket for this source and kind.
    pub events: i64,
}

/// Handle to the TimescaleDB pool.
///
/// Cloning is cheap: clones share the same pool.
#[derive(Clone)]
#[must_use = "dropping Store closes the connection pool"]
pub struct Store {
    pool: PgPool,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Store").finish_non_exhaustive()
    }
}

impl Store {
    /// Opens a pool to TimescaleDB.
    ///
    /// `url` is a Postgres connection string. Local compose uses
    /// [`LOCAL_DATABASE_URL`].
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when `url` is invalid or the server
    /// cannot be reached.
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        let options = PgConnectOptions::from_str(url)?.application_name("ledger-store");
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await?;
        Ok(Self { pool })
    }

    /// Applies the embedded SQL migrations.
    ///
    /// Migrations create the `events` hypertable and the `verdicts` table.
    /// Running this again does nothing once the schema is current.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Migrate`] when a migration statement fails.
    pub async fn migrate(&self) -> Result<(), StoreError> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }

    /// Inserts one event.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the insert fails.
    pub async fn insert_event(&self, event: &Event) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO events (id, observed_at, source, kind, data_class, payload)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(event.id)
        .bind(event.observed_at)
        .bind(event.source.as_str())
        .bind(event.kind.as_str())
        .bind(event.data_class.as_str())
        .bind(&event.payload)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Reads the latest event with `id`.
    ///
    /// Returns `Ok(None)` when no row has that id.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn fetch_event(&self, id: Uuid) -> Result<Option<Event>, StoreError> {
        let event = sqlx::query_as::<_, Event>(
            "SELECT id, observed_at, source, kind, data_class, payload
             FROM events
             WHERE id = $1
             ORDER BY observed_at DESC
             LIMIT 1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(event)
    }

    /// Inserts one verdict.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the insert fails.
    pub async fn insert_verdict(&self, verdict: &Verdict) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO verdicts
                (id, recorded_at, outcome, spec_id, reason_code, evidence)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(verdict.id)
        .bind(verdict.recorded_at)
        .bind(verdict.outcome.as_str())
        .bind(verdict.spec_id.as_str())
        .bind(verdict.reason_code.as_deref())
        .bind(&verdict.evidence)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Reads the verdict with `id`.
    ///
    /// Returns `Ok(None)` when no row has that id.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails, or
    /// [`StoreError::InvalidOutcome`] when the stored label is not a known
    /// outcome.
    pub async fn fetch_verdict(&self, id: Uuid) -> Result<Option<Verdict>, StoreError> {
        let row = sqlx::query_as::<_, VerdictRow>(
            "SELECT id, recorded_at, outcome, spec_id, reason_code, evidence
             FROM verdicts
             WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(Verdict::try_from).transpose()
    }

    /// Counts events per [`time_bucket`](https://docs.timescale.com/api/latest/hyperfunctions/time_bucket/)
    /// inside `[start, end)`.
    ///
    /// `source` limits the count to one source. `None` counts every source.
    /// The demo query is a one-minute bucket over the last hour, grouped by
    /// source and kind:
    ///
    /// ```sql
    /// SELECT time_bucket('1 minute', observed_at) AS bucket,
    ///        source,
    ///        kind,
    ///        count(*) AS events
    /// FROM events
    /// WHERE observed_at >= now() - interval '1 hour'
    ///   AND observed_at < now()
    /// GROUP BY bucket, source, kind
    /// ORDER BY bucket, source, kind
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn window_counts(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        width: BucketWidth,
        source: Option<&str>,
    ) -> Result<Vec<WindowCount>, StoreError> {
        let counts = match source {
            Some(source) => {
                sqlx::query_as::<_, WindowCount>(sql_filtered(width))
                    .bind(start)
                    .bind(end)
                    .bind(source)
                    .fetch_all(&self.pool)
                    .await?
            }
            None => {
                sqlx::query_as::<_, WindowCount>(sql_all(width))
                    .bind(start)
                    .bind(end)
                    .fetch_all(&self.pool)
                    .await?
            }
        };
        Ok(counts)
    }
}

fn sql_all(width: BucketWidth) -> &'static str {
    match width {
        BucketWidth::Minute => MINUTE_ALL,
        BucketWidth::Hour => HOUR_ALL,
    }
}

fn sql_filtered(width: BucketWidth) -> &'static str {
    match width {
        BucketWidth::Minute => MINUTE_SOURCE,
        BucketWidth::Hour => HOUR_SOURCE,
    }
}

const MINUTE_ALL: &str = "\
SELECT time_bucket('1 minute'::interval, observed_at) AS bucket,
       source,
       kind,
       count(*)::bigint AS events
FROM events
WHERE observed_at >= $1
  AND observed_at < $2
GROUP BY bucket, source, kind
ORDER BY bucket, source, kind";

const MINUTE_SOURCE: &str = "\
SELECT time_bucket('1 minute'::interval, observed_at) AS bucket,
       source,
       kind,
       count(*)::bigint AS events
FROM events
WHERE observed_at >= $1
  AND observed_at < $2
  AND source = $3
GROUP BY bucket, source, kind
ORDER BY bucket, source, kind";

const HOUR_ALL: &str = "\
SELECT time_bucket('1 hour'::interval, observed_at) AS bucket,
       source,
       kind,
       count(*)::bigint AS events
FROM events
WHERE observed_at >= $1
  AND observed_at < $2
GROUP BY bucket, source, kind
ORDER BY bucket, source, kind";

const HOUR_SOURCE: &str = "\
SELECT time_bucket('1 hour'::interval, observed_at) AS bucket,
       source,
       kind,
       count(*)::bigint AS events
FROM events
WHERE observed_at >= $1
  AND observed_at < $2
  AND source = $3
GROUP BY bucket, source, kind
ORDER BY bucket, source, kind";

#[cfg(test)]
mod tests {
    use super::*;

    fn timestamp() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("fixed timestamp")
    }

    /// Each test opens its own pool. A pool is bound to the tokio runtime that
    /// created it, and `#[tokio::test]` gives every test its own runtime.
    async fn test_store() -> Store {
        let store = Store::connect(&database_url())
            .await
            .expect("connect to local timescale (just db-up)");
        store.migrate().await.expect("apply migrations");
        store
    }

    #[test]
    fn verdict_outcome_parses_spec_labels_only() {
        assert_eq!(
            VerdictOutcome::try_from("Satisfy").expect("satisfy"),
            VerdictOutcome::Satisfy
        );
        assert_eq!(
            VerdictOutcome::try_from("Violate").expect("violate"),
            VerdictOutcome::Violate
        );
        assert_eq!(
            VerdictOutcome::try_from("Inconclusive").expect("inconclusive"),
            VerdictOutcome::Inconclusive
        );
        assert!(VerdictOutcome::try_from("pass").is_err());
        assert_eq!(VerdictOutcome::Satisfy.as_str(), "Satisfy");
        assert_eq!(VerdictOutcome::Violate.as_str(), "Violate");
        assert_eq!(VerdictOutcome::Inconclusive.as_str(), "Inconclusive");
    }

    #[tokio::test]
    async fn events_relation_is_a_hypertable() {
        let store = test_store().await;
        let name: Option<String> = sqlx::query_scalar(
            "SELECT hypertable_name::text
             FROM timescaledb_information.hypertables
             WHERE hypertable_schema = 'public'
               AND hypertable_name = 'events'",
        )
        .fetch_optional(&store.pool)
        .await
        .expect("lookup events hypertable");
        assert_eq!(name.as_deref(), Some("events"));
    }

    #[tokio::test]
    async fn insert_event_round_trip_reads_the_same_row() {
        let store = test_store().await;
        let event = Event {
            id: Uuid::new_v4(),
            observed_at: timestamp(),
            source: "exchange".to_owned(),
            kind: "bill.pay".to_owned(),
            data_class: "tenant".to_owned(),
            payload: serde_json::json!({
                "bill_id": "bill-1",
                "amount": "500.00"
            }),
        };

        store.insert_event(&event).await.expect("insert event");
        let loaded = store
            .fetch_event(event.id)
            .await
            .expect("fetch event")
            .expect("inserted event");
        assert_eq!(loaded, event);

        let missing = store
            .fetch_event(Uuid::new_v4())
            .await
            .expect("fetch missing event");
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn insert_verdict_round_trip_reads_the_same_row() {
        let store = test_store().await;
        let verdict = Verdict {
            id: Uuid::new_v4(),
            recorded_at: timestamp(),
            outcome: VerdictOutcome::Violate,
            spec_id: "no-second-payment".to_owned(),
            reason_code: Some("second-payment".to_owned()),
            evidence: serde_json::json!([
                {"kind": "bill.pay", "bill_id": "bill-1"}
            ]),
        };

        store
            .insert_verdict(&verdict)
            .await
            .expect("insert verdict");
        let loaded = store
            .fetch_verdict(verdict.id)
            .await
            .expect("fetch verdict")
            .expect("inserted verdict");
        assert_eq!(loaded, verdict);

        let missing = store
            .fetch_verdict(Uuid::new_v4())
            .await
            .expect("fetch missing verdict");
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn verdicts_reject_outcome_pass() {
        let store = test_store().await;
        let error = sqlx::query(
            "INSERT INTO verdicts (id, recorded_at, outcome, spec_id, evidence)
             VALUES ($1, $2, 'pass', 'spec', '[]'::jsonb)",
        )
        .bind(Uuid::new_v4())
        .bind(timestamp())
        .execute(&store.pool)
        .await
        .expect_err("pass is not a verdict outcome");

        let message = error.to_string();
        assert!(
            message.contains("verdicts_outcome_known"),
            "unexpected error: {message}"
        );
    }

    #[tokio::test]
    async fn window_counts_return_per_bucket_counts() {
        let store = test_store().await;
        let source = format!("window-{}", Uuid::new_v4());
        // Aligned to a minute so the next event falls in the following bucket.
        let first = DateTime::from_timestamp(1_900_000_080, 0).expect("aligned minute");
        let second = first + chrono::Duration::seconds(60);
        let outside = first - chrono::Duration::hours(2);

        for (observed_at, kind) in [
            (first, "agent.decide"),
            (first, "agent.decide"),
            (second, "exchange.submit_order"),
            (outside, "agent.decide"),
        ] {
            store
                .insert_event(&Event {
                    id: Uuid::new_v4(),
                    observed_at,
                    source: source.clone(),
                    kind: kind.to_owned(),
                    data_class: "tenant".to_owned(),
                    payload: serde_json::json!({}),
                })
                .await
                .expect("insert window event");
        }

        let counts = store
            .window_counts(
                first,
                second + chrono::Duration::minutes(1),
                BucketWidth::Minute,
                Some(&source),
            )
            .await
            .expect("window query");

        assert_eq!(counts.len(), 2);
        assert_eq!(counts[0].bucket, first);
        assert_eq!(counts[0].kind, "agent.decide");
        assert_eq!(counts[0].events, 2);
        assert_eq!(counts[1].bucket, second);
        assert_eq!(counts[1].kind, "exchange.submit_order");
        assert_eq!(counts[1].events, 1);
        assert!(counts.iter().all(|count| count.source == source));
    }
}
