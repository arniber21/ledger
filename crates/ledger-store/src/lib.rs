//! TimescaleDB storage for live events and verdicts.
//!
//! Events live in a hypertable partitioned on `observed_at`. Verdicts are a
//! regular table. An outcome is `Satisfy`, `Violate`, or `Inconclusive`.
//! `jev_calibration` counts stored inbox reviews by Jev score band.

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

    /// A stored triage status was not New, Reviewing, Confirmed, or Dismissed.
    #[error("invalid triage status: {0}")]
    InvalidStatus(String),

    /// A Jev judgment was stored with no question id.
    #[error("jev question is empty")]
    EmptyQuestion,
}

/// `events.kind` for an inbox review. The calibration view reads this kind.
pub const REVIEW_EVENT_KIND: &str = "verdict.review";

/// `events.kind` for a stored Jev score. The calibration view reads this kind.
pub const JUDGMENT_EVENT_KIND: &str = "jev.judgment";

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

/// Desk status for one verdict. A missing row means [`TriageStatus::New`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriageStatus {
    /// The verdict is waiting for a person.
    New,
    /// Someone is looking at the verdict.
    Reviewing,
    /// Marked reviewed. This is needs-review work that a person closed.
    Confirmed,
    /// Removed from the queue.
    Dismissed,
}

impl TriageStatus {
    /// Returns the label stored in `verdict_triage.status`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Reviewing => "Reviewing",
            Self::Confirmed => "Confirmed",
            Self::Dismissed => "Dismissed",
        }
    }

    /// Parses a stored label. `fraud` and `pass` are not statuses.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "New" => Some(Self::New),
            "Reviewing" => Some(Self::Reviewing),
            "Confirmed" => Some(Self::Confirmed),
            "Dismissed" => Some(Self::Dismissed),
            _ => None,
        }
    }
}

/// One stored triage row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Triage {
    pub verdict_id: Uuid,
    pub status: TriageStatus,
    pub assignee: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub updated_by: String,
}

/// A triage write plus the audit event id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriageChange {
    pub triage: Triage,
    pub event_id: Uuid,
}

/// Result of asking the store to record triage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriageWrite {
    /// The verdict exists and the status was written.
    Stored(TriageChange),
    /// No verdict has this id, so nothing was written.
    UnknownVerdict,
}

/// Result of asking the store to record a Jev score for a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JudgmentWrite {
    /// The verdict exists and a `jev.judgment` event was stored.
    Stored {
        /// Id of the hypertable row.
        event_id: Uuid,
    },
    /// No verdict has this id, so nothing was written.
    UnknownVerdict,
}

/// One row of `jev_calibration`.
///
/// `confirmed` and `dismissed` are inbox labels. They are not verdict
/// outcomes. `unknown` means the stored score was missing or not usable.
/// That band is inconclusive, not a pass. `reviewed` is the sample size.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CalibrationRow {
    /// Jev question id stored on the judgment event.
    pub question: String,
    /// Score band, or `unknown` when the score is not a probability in `[0, 1]`.
    pub band: String,
    /// Latest review for the verdict was Confirmed.
    pub confirmed: i64,
    /// Latest review for the verdict was Dismissed.
    pub dismissed: i64,
    /// `confirmed` plus `dismissed`. This is the sample, not a rate.
    pub reviewed: i64,
}

#[derive(sqlx::FromRow)]
struct TriageRow {
    verdict_id: Uuid,
    status: String,
    assignee: Option<String>,
    updated_at: DateTime<Utc>,
    updated_by: String,
}

impl TryFrom<TriageRow> for Triage {
    type Error = StoreError;

    fn try_from(row: TriageRow) -> Result<Self, Self::Error> {
        let status = TriageStatus::parse(&row.status)
            .ok_or_else(|| StoreError::InvalidStatus(row.status))?;
        Ok(Self {
            verdict_id: row.verdict_id,
            status,
            assignee: row.assignee,
            updated_at: row.updated_at,
            updated_by: row.updated_by,
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

    /// Creates an empty, migrated database named `ledger_scratch_*` on the same
    /// server as `admin_url` and connects to it.
    ///
    /// Integration tests use this so that a running stack on the shared
    /// database never sees their events. Without it, the live verdict engine
    /// inserts the same deterministic verdict first and the test publishes
    /// nothing. Pair it with [`Store::drop_scratch`].
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] when the server is unreachable, the database
    /// cannot be created, or a migration fails.
    pub async fn create_scratch(admin_url: &str) -> Result<(Self, String), StoreError> {
        let name = format!("ledger_scratch_{}", Uuid::new_v4().simple());
        let admin = Self::connect(admin_url).await?;
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&admin.pool)
            .await?;
        admin.pool.close().await;
        let options = PgConnectOptions::from_str(admin_url)?
            .database(&name)
            .application_name("ledger-store");
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok((store, name))
    }

    /// Closes this pool and drops a database made by [`Store::create_scratch`].
    /// Refuses any name without the `ledger_scratch_` prefix.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the drop fails.
    pub async fn drop_scratch(self, admin_url: &str, name: &str) -> Result<(), StoreError> {
        if !name.starts_with("ledger_scratch_")
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Ok(());
        }
        self.pool.close().await;
        let admin = Self::connect(admin_url).await?;
        sqlx::query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)"))
            .execute(&admin.pool)
            .await?;
        admin.pool.close().await;
        Ok(())
    }

    /// Connects to the `ledger_test` database, creating and migrating it.
    ///
    /// Tests use this instead of [`Store::connect`] so they never write
    /// fixture rows (`no-second-payment`, 2023 timestamps) into the live demo
    /// database. The server is the one `DATABASE_URL` names; only the
    /// database name changes.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the server is unreachable or the
    /// database cannot be created, and [`StoreError::Migrate`] on migration.
    pub async fn connect_test() -> Result<Self, StoreError> {
        let admin_url = database_url();
        let test_url = match admin_url.rfind('/') {
            Some(index) => format!("{}/ledger_test", &admin_url[..index]),
            None => format!("{admin_url}/ledger_test"),
        };
        let admin = Self::connect(&admin_url).await?;
        let exists: Option<i32> =
            sqlx::query_scalar("SELECT 1 FROM pg_database WHERE datname = 'ledger_test'")
                .fetch_optional(&admin.pool)
                .await?;
        if exists.is_none() {
            // Parallel tests can race here; losing the race is fine.
            let _ = sqlx::query("CREATE DATABASE ledger_test")
                .execute(&admin.pool)
                .await;
        }
        admin.pool.close().await;
        let store = Self::connect(&test_url).await?;
        store.migrate().await?;
        Ok(store)
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

    /// Deletes events, verdicts, triage, and ranks.
    ///
    /// The schema stays. The next scene starts from an empty hypertable.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the truncate fails.
    pub async fn truncate_runtime(&self) -> Result<(), StoreError> {
        sqlx::query("TRUNCATE TABLE verdict_rank, verdict_triage, verdicts, events")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Deletes one scene's events and the verdicts for `bindings`.
    ///
    /// Events match `payload.attributes.scene`. Verdicts match `evidence.scene`
    /// or `evidence.binding`. Other scenes stay.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when a delete fails.
    pub async fn clear_scene(&self, scene: &str, bindings: &[String]) -> Result<(), StoreError> {
        let verdicts = "SELECT id FROM verdicts
            WHERE evidence->>'scene' = $1
               OR evidence->>'binding' = ANY($2)";
        sqlx::query(&format!(
            "DELETE FROM verdict_rank WHERE verdict_id IN ({verdicts})"
        ))
        .bind(scene)
        .bind(bindings)
        .execute(&self.pool)
        .await?;
        sqlx::query(&format!(
            "DELETE FROM verdict_triage WHERE verdict_id IN ({verdicts})"
        ))
        .bind(scene)
        .bind(bindings)
        .execute(&self.pool)
        .await?;
        sqlx::query(&format!("DELETE FROM verdicts WHERE id IN ({verdicts})"))
            .bind(scene)
            .bind(bindings)
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "DELETE FROM events
             WHERE payload->'attributes'->>'scene' = $1
                OR payload->>'scene' = $1",
        )
        .bind(scene)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Reads up to `limit` events of `kind` inside `[start, end)`.
    ///
    /// The time bounds let Timescale skip chunks. A kind-only scan reads every
    /// chunk.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn events_by_kind_between(
        &self,
        kind: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<Event>, StoreError> {
        let events = sqlx::query_as::<_, Event>(
            "SELECT id, observed_at, source, kind, data_class, payload
             FROM events
             WHERE kind = $1
               AND observed_at >= $2
               AND observed_at < $3
             ORDER BY observed_at ASC, id ASC
             LIMIT $4",
        )
        .bind(kind)
        .bind(start)
        .bind(end)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(events)
    }

    /// Merges `signatures` into a verdict's evidence document.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the update fails.
    /// Sets `payload.predecessors` so a later poll can see the causal edge.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the update fails.
    pub async fn set_predecessors(
        &self,
        id: Uuid,
        predecessors: &[String],
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE events
             SET payload = jsonb_set(payload, '{predecessors}', $2::jsonb, true)
             WHERE id = $1",
        )
        .bind(id)
        .bind(serde_json::json!(predecessors))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn attach_signatures(
        &self,
        id: Uuid,
        signatures: &[String],
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE verdicts
             SET evidence = evidence || jsonb_build_object('signatures', $2::jsonb)
             WHERE id = $1",
        )
        .bind(id)
        .bind(serde_json::json!(signatures))
        .execute(&self.pool)
        .await?;
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

    /// Reads events in `[start, end)`, ordered by `observed_at`.
    ///
    /// This is the row read next to [`Self::window_counts`]. Both filter the
    /// hypertable on `observed_at`. `window_counts` aggregates; this returns
    /// the events a verdict monitor steps.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn events_in_span(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<Event>, StoreError> {
        let limit = limit.clamp(1, 1_000);
        let events = sqlx::query_as::<_, Event>(
            "SELECT id, observed_at, source, kind, data_class, payload
             FROM events
             WHERE observed_at >= $1
               AND observed_at < $2
             ORDER BY observed_at ASC, id ASC
             LIMIT $3",
        )
        .bind(start)
        .bind(end)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(events)
    }

    /// Next page of [`Self::events_in_span`] after `(observed_at, id)`.
    ///
    /// `after` must fall inside `[start, end)`. Rows stay ordered by
    /// `observed_at`, then `id`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn events_in_span_after(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        after: (DateTime<Utc>, Uuid),
        limit: i64,
    ) -> Result<Vec<Event>, StoreError> {
        let limit = limit.clamp(1, 1_000);
        let events = sqlx::query_as::<_, Event>(
            "SELECT id, observed_at, source, kind, data_class, payload
             FROM events
             WHERE observed_at >= $1
               AND observed_at < $2
               AND (observed_at, id) > ($3, $4)
             ORDER BY observed_at ASC, id ASC
             LIMIT $5",
        )
        .bind(start)
        .bind(end)
        .bind(after.0)
        .bind(after.1)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(events)
    }

    /// The latest `limit` events, oldest first.
    ///
    /// `limit` is clamped to 1..=1000. Ordering matches the hypertable primary
    /// key `(id, observed_at)` so a later [`events_after`](Self::events_after)
    /// call can continue from the last row.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn recent_events(&self, limit: i64) -> Result<Vec<Event>, StoreError> {
        let events = sqlx::query_as::<_, Event>(
            "SELECT id, observed_at, source, kind, data_class, payload
             FROM (
                 SELECT id, observed_at, source, kind, data_class, payload
                 FROM events
                 ORDER BY observed_at DESC, id DESC
                 LIMIT $1
             ) AS recent
             ORDER BY observed_at ASC, id ASC",
        )
        .bind(clamp_limit(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(events)
    }

    /// The latest `limit` ingested spans, oldest first.
    ///
    /// A span row has `payload.span`. Reviews, judgments, and rows inserted
    /// with an empty payload are not spans. `observed_at` must be no more than
    /// five minutes ahead of the database clock, so a fixture dated years
    /// ahead cannot fill this page.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn recent_span_events(&self, limit: i64) -> Result<Vec<Event>, StoreError> {
        let events = sqlx::query_as::<_, Event>(
            "SELECT id, observed_at, source, kind, data_class, payload
             FROM (
                 SELECT id, observed_at, source, kind, data_class, payload
                 FROM events
                 WHERE payload ? 'span'
                   AND observed_at <= now() + interval '5 minutes'
                 ORDER BY observed_at DESC, id DESC
                 LIMIT $1
             ) AS recent
             ORDER BY observed_at ASC, id ASC",
        )
        .bind(clamp_limit(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(events)
    }

    /// Span rows strictly after `(observed_at, id)`, oldest first.
    ///
    /// The same span predicate as [`Self::recent_span_events`].
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn span_events_after(
        &self,
        after: (DateTime<Utc>, Uuid),
        limit: i64,
    ) -> Result<Vec<Event>, StoreError> {
        let events = sqlx::query_as::<_, Event>(
            "SELECT id, observed_at, source, kind, data_class, payload
             FROM events
             WHERE payload ? 'span'
               AND observed_at <= now() + interval '5 minutes'
               AND (observed_at, id) > ($1, $2)
             ORDER BY observed_at ASC, id ASC
             LIMIT $3",
        )
        .bind(after.0)
        .bind(after.1)
        .bind(clamp_limit(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(events)
    }

    /// Span rows in `[start, end)`.
    ///
    /// Used to pick up the rest of a trace whose parent closed inside the
    /// feed page and whose children closed earlier.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn span_events_between(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<Event>, StoreError> {
        let limit = limit.clamp(1, 2_000);
        let events = sqlx::query_as::<_, Event>(
            "SELECT id, observed_at, source, kind, data_class, payload
             FROM events
             WHERE payload ? 'span'
               AND observed_at >= $1
               AND observed_at < $2
             ORDER BY observed_at ASC, id ASC
             LIMIT $3",
        )
        .bind(start)
        .bind(end)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(events)
    }

    /// Reads the next page of events after `(observed_at, id)`.
    ///
    /// `None` starts at the oldest row. Ordering matches [`Self::events_in_span`].
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn events_after(
        &self,
        after: Option<(DateTime<Utc>, Uuid)>,
        limit: i64,
    ) -> Result<Vec<Event>, StoreError> {
        let limit = limit.clamp(1, 1_000);
        let events = if let Some((observed_at, id)) = after {
            sqlx::query_as::<_, Event>(
                "SELECT id, observed_at, source, kind, data_class, payload
                 FROM events
                 WHERE (observed_at, id) > ($1, $2)
                 ORDER BY observed_at ASC, id ASC
                 LIMIT $3",
            )
            .bind(observed_at)
            .bind(id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, Event>(
                "SELECT id, observed_at, source, kind, data_class, payload
                 FROM events
                 ORDER BY observed_at ASC, id ASC
                 LIMIT $1",
            )
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(events)
    }

    /// Inserts a verdict when its id is new.
    ///
    /// Returns `true` when the row was inserted. A repeat id is left as stored.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the insert fails.
    pub async fn insert_verdict_if_absent(&self, verdict: &Verdict) -> Result<bool, StoreError> {
        let inserted = sqlx::query(
            "INSERT INTO verdicts
                (id, recorded_at, outcome, spec_id, reason_code, evidence)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(verdict.id)
        .bind(verdict.recorded_at)
        .bind(verdict.outcome.as_str())
        .bind(verdict.spec_id.as_str())
        .bind(verdict.reason_code.as_deref())
        .bind(&verdict.evidence)
        .execute(&self.pool)
        .await?;
        Ok(inserted.rows_affected() == 1)
    }

    /// Reads the newest verdicts, newest first.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails, or
    /// [`StoreError::InvalidOutcome`] when a stored label is not a known outcome.
    pub async fn list_verdicts(&self, limit: i64) -> Result<Vec<Verdict>, StoreError> {
        let limit = limit.clamp(1, 1_000);
        let rows = sqlx::query_as::<_, VerdictRow>(
            "SELECT id, recorded_at, outcome, spec_id, reason_code, evidence
             FROM verdicts
             ORDER BY recorded_at DESC, id DESC
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Verdict::try_from).collect()
    }

    /// Writes triage status for a stored verdict and appends a review event.
    ///
    /// `UnknownVerdict` means `verdict_id` is not in `verdicts`. No event is
    /// written in that case. A later call for the same verdict replaces the
    /// status. The event source is `inbox` and the kind is `verdict.review`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when a query fails.
    pub async fn record_triage(
        &self,
        verdict_id: Uuid,
        status: TriageStatus,
        assignee: Option<&str>,
        actor: &str,
    ) -> Result<TriageWrite, StoreError> {
        if self.fetch_verdict(verdict_id).await?.is_none() {
            return Ok(TriageWrite::UnknownVerdict);
        }
        let updated_at = Utc::now();
        let event_id = Uuid::new_v4();
        let actor = {
            let trimmed = actor.trim();
            if trimmed.is_empty() {
                "me"
            } else {
                trimmed
            }
        };
        let assignee = assignee
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO verdict_triage
                (verdict_id, status, assignee, updated_at, updated_by)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (verdict_id) DO UPDATE
             SET status = EXCLUDED.status,
                 assignee = EXCLUDED.assignee,
                 updated_at = EXCLUDED.updated_at,
                 updated_by = EXCLUDED.updated_by",
        )
        .bind(verdict_id)
        .bind(status.as_str())
        .bind(assignee.as_deref())
        .bind(updated_at)
        .bind(actor)
        .execute(&mut *tx)
        .await?;
        let payload = serde_json::json!({
            "verdict_id": verdict_id.to_string(),
            "status": status.as_str(),
            "assignee": assignee,
            "actor": actor,
        });
        sqlx::query(&format!(
            "INSERT INTO events (id, observed_at, source, kind, data_class, payload)
             VALUES ($1, $2, 'inbox', '{REVIEW_EVENT_KIND}', 'tenant', $3)"
        ))
        .bind(event_id)
        .bind(updated_at)
        .bind(&payload)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(TriageWrite::Stored(TriageChange {
            triage: Triage {
                verdict_id,
                status,
                assignee,
                updated_at,
                updated_by: actor.to_owned(),
            },
            event_id,
        }))
    }

    /// Reads triage rows, newest update first.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails, or
    /// [`StoreError::InvalidStatus`] when a stored label is not a known status.
    pub async fn list_triage(&self) -> Result<Vec<Triage>, StoreError> {
        let rows = sqlx::query_as::<_, TriageRow>(
            "SELECT verdict_id, status, assignee, updated_at, updated_by
             FROM verdict_triage
             ORDER BY updated_at DESC, verdict_id ASC
             LIMIT 1000",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Triage::try_from).collect()
    }

    /// Inserts raw ranking scores when this verdict has none yet.
    ///
    /// Returns `true` when the row was inserted. A repeat id is left as stored
    /// so a later weight change does not call Jev again. NULL dimensions are
    /// Unknown, not a priority of zero.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the insert fails, including when
    /// `verdict_id` is not in `verdicts`.
    pub async fn insert_rank_if_absent(&self, rank: &VerdictRank) -> Result<bool, StoreError> {
        let inserted = sqlx::query(
            "INSERT INTO verdict_rank
                (verdict_id, recorded_at, request, response, money, evidence, ambiguity, intent)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (verdict_id) DO NOTHING",
        )
        .bind(rank.verdict_id)
        .bind(rank.recorded_at)
        .bind(&rank.request)
        .bind(&rank.response)
        .bind(rank.money)
        .bind(rank.evidence)
        .bind(rank.ambiguity)
        .bind(rank.intent)
        .execute(&self.pool)
        .await?;
        Ok(inserted.rows_affected() == 1)
    }

    /// Reads stored ranking scores, newest first.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn list_ranks(&self) -> Result<Vec<VerdictRank>, StoreError> {
        let rows = sqlx::query_as::<_, VerdictRank>(
            "SELECT verdict_id, recorded_at, request, response, money, evidence, ambiguity, intent
             FROM verdict_rank
             ORDER BY recorded_at DESC, verdict_id ASC
             LIMIT 1000",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Stores one Jev score for a verdict as a `jev.judgment` event.
    ///
    /// `probability` is the score already returned by Jev. `None`, a non-finite
    /// value, or a value outside `[0, 1]` is stored as a missing score. The
    /// calibration view reports that as `unknown`, which is inconclusive, not
    /// a pass. This method does not call Jev and does not change the 0.80
    /// thresholds.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::EmptyQuestion`] when `question` is blank, or
    /// [`StoreError::Database`] when a query fails.
    pub async fn record_jev_judgment(
        &self,
        verdict_id: Uuid,
        question: &str,
        probability: Option<f64>,
    ) -> Result<JudgmentWrite, StoreError> {
        let question = question.trim();
        if question.is_empty() {
            return Err(StoreError::EmptyQuestion);
        }
        if self.fetch_verdict(verdict_id).await?.is_none() {
            return Ok(JudgmentWrite::UnknownVerdict);
        }
        let event_id = Uuid::new_v4();
        let observed_at = Utc::now();
        let score = usable_probability(probability);
        let mut payload = serde_json::json!({
            "verdict_id": verdict_id.to_string(),
            "question": question,
            "probability": score,
        });
        if score.is_none() {
            payload["evidence"] = serde_json::json!("inconclusive");
            payload["reason"] = serde_json::json!("jev score is not usable");
        }
        sqlx::query(&format!(
            "INSERT INTO events (id, observed_at, source, kind, data_class, payload)
             VALUES ($1, $2, 'jev', '{JUDGMENT_EVENT_KIND}', 'tenant', $3)"
        ))
        .bind(event_id)
        .bind(observed_at)
        .bind(&payload)
        .execute(&self.pool)
        .await?;
        Ok(JudgmentWrite::Stored { event_id })
    }

    /// Reads the calibration view.
    ///
    /// Rows come from stored `verdict.review` events joined to stored
    /// `jev.judgment` events. The definition is
    /// `migrations/0003_jev_calibration.sql`. An empty vec means no reviewed
    /// judgment is stored. That is not a pass.
    ///
    /// ```sql
    /// SELECT question, band, confirmed, dismissed, reviewed
    /// FROM jev_calibration
    /// ORDER BY question, band
    /// LIMIT 1000
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn calibration(&self) -> Result<Vec<CalibrationRow>, StoreError> {
        let rows = sqlx::query_as::<_, CalibrationRow>(
            "SELECT question, band, confirmed, dismissed, reviewed
             FROM jev_calibration
             ORDER BY question, band
             LIMIT 1000",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Counters for the dashboard, as of `now`. Events dated after `now` plus
    /// one day are not counted.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn metric_totals(&self, now: DateTime<Utc>) -> Result<MetricTotals, StoreError> {
        let ceiling = now + chrono::Duration::days(1);
        let totals = sqlx::query_as::<_, MetricTotals>(
            "SELECT
                 (SELECT count(*) FROM events
                   WHERE observed_at <= $1
                     AND (source = 'exchange' OR source LIKE 'agent%'))::bigint AS events,
                 (SELECT count(*) FROM events
                   WHERE observed_at > $2 AND observed_at <= $3
                     AND (source = 'exchange' OR source LIKE 'agent%'))::bigint AS events_last_minute,
                 (SELECT count(DISTINCT payload #>> '{attributes,tx.signature}') FROM events
                   WHERE observed_at <= $1
                     AND COALESCE(payload #>> '{attributes,tx.signature}', '') <> '')::bigint AS settled_on_chain,
                 (SELECT count(DISTINCT source) FROM events
                   WHERE observed_at <= $1
                     AND (source = 'exchange' OR source LIKE 'agent%'))::bigint AS source_count,
                 (
                    (SELECT count(*) FROM verdict_rank)
                    + (SELECT count(*) FROM verdicts v
                        WHERE v.evidence #>> '{comparison,jev,outcome}' IS NOT NULL
                          AND NOT EXISTS (
                            SELECT 1 FROM verdict_rank r WHERE r.verdict_id = v.id
                          ))
                 )::bigint AS jev_judgments,
                 (
                    (SELECT count(*) FROM verdict_rank
                      WHERE money IS NULL AND evidence IS NULL
                        AND ambiguity IS NULL AND intent IS NULL)
                    + (SELECT count(*) FROM verdicts v
                        WHERE v.evidence #>> '{comparison,jev,outcome}' = 'Unknown'
                          AND NOT EXISTS (
                            SELECT 1 FROM verdict_rank r WHERE r.verdict_id = v.id
                          ))
                 )::bigint AS jev_unknown",
        )
        .bind(ceiling)
        .bind(now - chrono::Duration::minutes(1))
        .bind(now + chrono::Duration::seconds(5))
        .fetch_one(&self.pool)
        .await?;
        Ok(totals)
    }

    /// Events per `seconds`-wide bucket in `[start, end)`, by source class.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn rate_buckets(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        seconds: i32,
    ) -> Result<Vec<RateBucket>, StoreError> {
        let rows = sqlx::query_as::<_, RateBucket>(
            "SELECT time_bucket(make_interval(secs => $3), observed_at) AS bucket,
                    (count(*) FILTER (WHERE source LIKE 'agent%'))::bigint AS agent,
                    (count(*) FILTER (WHERE source = 'exchange'))::bigint AS exchange,
                    (count(*) FILTER (WHERE payload->'attributes' ? 'tx.signature'))::bigint AS solana
             FROM events
             WHERE observed_at >= $1 AND observed_at < $2
               AND (source = 'exchange' OR source LIKE 'agent%')
             GROUP BY bucket
             ORDER BY bucket",
        )
        .bind(start)
        .bind(end)
        .bind(f64::from(seconds))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Needs-review and inconclusive verdicts recorded per minute in `[start, end)`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    pub async fn attention_buckets(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<AttentionBucket>, StoreError> {
        let rows = sqlx::query_as::<_, AttentionBucket>(
            "SELECT date_trunc('minute', recorded_at) AS bucket,
                    (count(*) FILTER (WHERE outcome = 'Violate'))::bigint AS violate,
                    (count(*) FILTER (WHERE outcome = 'Inconclusive'))::bigint AS inconclusive
             FROM verdicts
             WHERE recorded_at >= $1 AND recorded_at < $2
             GROUP BY bucket
             ORDER BY bucket",
        )
        .bind(start)
        .bind(end)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Rows of [`Self::calibration`] for one question id.
    ///
    /// An empty vec means this question has no reviewed judgment. That is not
    /// a pass.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Database`] when the query fails.
    #[cfg(test)]
    pub(crate) async fn calibration_for(
        &self,
        question: &str,
    ) -> Result<Vec<CalibrationRow>, StoreError> {
        let rows = sqlx::query_as::<_, CalibrationRow>(
            "SELECT question, band, confirmed, dismissed, reviewed
             FROM jev_calibration
             WHERE question = $1
             ORDER BY band",
        )
        .bind(question)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}

fn usable_probability(value: Option<f64>) -> Option<f64> {
    value.filter(|score| score.is_finite() && (0.0..=1.0).contains(score))
}

/// Raw Jev scores for one verdict. NULL dimensions are Unknown.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct VerdictRank {
    pub verdict_id: Uuid,
    pub recorded_at: DateTime<Utc>,
    pub request: serde_json::Value,
    pub response: Option<serde_json::Value>,
    pub money: Option<f64>,
    pub evidence: Option<f64>,
    pub ambiguity: Option<f64>,
    pub intent: Option<f64>,
}

/// Totals behind the dashboard counters. Rows dated past the ceiling are
/// ignored so a bad clock cannot inflate them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, sqlx::FromRow)]
pub struct MetricTotals {
    /// Trace events from agents and the exchange.
    pub events: i64,
    /// Trace events observed in the last minute.
    pub events_last_minute: i64,
    /// Distinct Solana transaction signatures on stored events.
    pub settled_on_chain: i64,
    /// Distinct agent and exchange sources.
    pub source_count: i64,
    /// Stored Jev scores, plus comparison cards that have no score row.
    pub jev_judgments: i64,
    /// Jev calls with no numeric dimension, plus comparison cards of `Unknown`.
    pub jev_unknown: i64,
}

/// Events in one time bucket, by source class.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct RateBucket {
    /// Bucket start.
    pub bucket: DateTime<Utc>,
    /// Events from `agent*` sources.
    pub agent: i64,
    /// Events from the exchange.
    pub exchange: i64,
    /// Events with a Solana transaction signature.
    pub solana: i64,
}

/// Verdicts recorded in one bucket.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct AttentionBucket {
    /// Bucket start.
    pub bucket: DateTime<Utc>,
    /// Needs-review verdicts.
    pub violate: i64,
    /// Inconclusive verdicts.
    pub inconclusive: i64,
}

/// Last hypertable row already delivered to a reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventCursor {
    pub observed_at: DateTime<Utc>,
    pub id: Uuid,
}

fn clamp_limit(limit: i64) -> i64 {
    limit.clamp(1, 1_000)
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
        let store = Store::connect_test()
            .await
            .expect("connect to ledger_test (just db-up)");
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
            spec_id: "ledger.bill.no-second-payment".to_owned(),
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
    async fn metric_totals_count_distinct_signatures_and_jev_cards() {
        let admin = database_url();
        let (store, name) = Store::create_scratch(&admin)
            .await
            .expect("scratch database");
        let now = Utc::now();
        let signature = "sig-repeat";
        for (index, sig) in [signature, signature, "sig-other"].into_iter().enumerate() {
            store
                .insert_event(&Event {
                    id: Uuid::new_v4(),
                    observed_at: now,
                    source: "exchange".to_owned(),
                    kind: "ledger.exchange.settled".to_owned(),
                    data_class: "tenant".to_owned(),
                    payload: serde_json::json!({
                        "attributes": {"tx.signature": sig, "n": index}
                    }),
                })
                .await
                .expect("insert settlement");
        }
        store
            .insert_event(&Event {
                id: Uuid::new_v4(),
                observed_at: now,
                source: "agent.payer".to_owned(),
                kind: "ledger.agent.pay_bill".to_owned(),
                data_class: "tenant".to_owned(),
                payload: serde_json::json!({"attributes": {}}),
            })
            .await
            .expect("insert payment");
        store
            .insert_verdict(&Verdict {
                id: Uuid::new_v4(),
                recorded_at: now,
                outcome: VerdictOutcome::Inconclusive,
                spec_id: "ledger.binding.vendor-alias".to_owned(),
                reason_code: Some("binding-comparison".to_owned()),
                evidence: serde_json::json!({
                    "comparison": {
                        "exact": {"outcome": "Different"},
                        "fuzzy": {"outcome": "Different"},
                        "jev": {"outcome": "Same"}
                    }
                }),
            })
            .await
            .expect("insert same card");
        store
            .insert_verdict(&Verdict {
                id: Uuid::new_v4(),
                recorded_at: now,
                outcome: VerdictOutcome::Inconclusive,
                spec_id: "ledger.binding.vendor-alias".to_owned(),
                reason_code: Some("binding-comparison".to_owned()),
                evidence: serde_json::json!({
                    "comparison": {"jev": {"outcome": "Unknown"}}
                }),
            })
            .await
            .expect("insert unknown card");

        let totals = store.metric_totals(now).await.expect("totals");
        assert_eq!(totals.settled_on_chain, 2, "{totals:?}");
        assert_eq!(totals.source_count, 2, "{totals:?}");
        assert_eq!(totals.events, 4, "{totals:?}");
        assert_eq!(totals.jev_judgments, 2, "{totals:?}");
        assert_eq!(totals.jev_unknown, 1, "{totals:?}");
        store
            .drop_scratch(&admin, &name)
            .await
            .expect("drop scratch");
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

    #[tokio::test]
    async fn events_after_reads_the_same_observed_at_order() {
        let store = test_store().await;
        let source = format!("cursor-{}", Uuid::new_v4());
        // A fresh window. Repeated runs share one database, and a fixed
        // timestamp fills the page limit with older rows.
        let skew = i64::try_from(Uuid::new_v4().as_u128() % 86_400_000_000).expect("skew");
        let first_at = DateTime::from_timestamp(4_500_000_000, 0).expect("time")
            + chrono::Duration::microseconds(skew);
        let mut ids = Vec::new();
        for offset in 0..3 {
            let id = Uuid::new_v4();
            ids.push(id);
            store
                .insert_event(&Event {
                    id,
                    observed_at: first_at + chrono::Duration::seconds(offset),
                    source: source.clone(),
                    kind: "ledger.agent.pay_bill".to_owned(),
                    data_class: "tenant".to_owned(),
                    payload: serde_json::json!({"n": offset}),
                })
                .await
                .expect("insert");
        }

        let page = store
            .events_in_span(first_at, first_at + chrono::Duration::seconds(3), 1_000)
            .await
            .expect("span");
        let ours: Vec<_> = page
            .into_iter()
            .filter(|event| event.source == source)
            .collect();
        assert_eq!(ours.len(), 3);
        assert_eq!(ours[0].id, ids[0]);
        assert_eq!(ours[2].id, ids[2]);

        let after = store
            .events_after(Some((ours[0].observed_at, ours[0].id)), 1_000)
            .await
            .expect("after");
        let rest: Vec<_> = after
            .into_iter()
            .filter(|event| event.source == source)
            .collect();
        assert_eq!(rest.len(), 2);
        assert_eq!(rest[0].id, ids[1]);
        assert_eq!(rest[1].id, ids[2]);
    }

    #[tokio::test]
    async fn recent_events_and_events_after_follow_the_primary_key() {
        let store = test_store().await;
        // Repeated runs share one database. A fixed future timestamp fills the
        // newest page, so this run sits ahead of rows already stored.
        let base = Utc::now() + chrono::Duration::days(80_000);
        let mut inserted = Vec::new();
        for offset in 0..3 {
            let event = Event {
                id: Uuid::new_v4(),
                observed_at: base + chrono::Duration::seconds(offset),
                source: "exchange".to_owned(),
                kind: "exchange.settle".to_owned(),
                data_class: "tenant".to_owned(),
                payload: serde_json::json!({
                    "span": "exchange.settle",
                    "span_id": offset.to_string(),
                }),
            };
            store.insert_event(&event).await.expect("insert event");
            inserted.push(event);
        }

        let recent = store.recent_events(20).await.expect("recent events");
        let recent_ids: Vec<_> = recent.iter().map(|event| event.id).collect();
        assert!(recent_ids.contains(&inserted[2].id));
        assert!(recent.windows(2).all(|pair| {
            (pair[0].observed_at, pair[0].id) <= (pair[1].observed_at, pair[1].id)
        }));

        let cursor = EventCursor {
            observed_at: inserted[0].observed_at,
            id: inserted[0].id,
        };
        let after = store
            .events_after(Some((cursor.observed_at, cursor.id)), 1_000)
            .await
            .expect("events after");
        let followed: Vec<_> = after
            .iter()
            .filter(|event| inserted.iter().any(|item| item.id == event.id))
            .map(|event| event.id)
            .collect();
        assert_eq!(followed, vec![inserted[1].id, inserted[2].id]);
        let second = after
            .iter()
            .find(|event| event.id == inserted[1].id)
            .expect("second event");
        assert_eq!(second.payload["span"], "exchange.settle");
    }

    #[test]
    fn triage_status_rejects_labels_outside_the_desk() {
        assert!(TriageStatus::parse("fraud").is_none());
        assert!(TriageStatus::parse("Fraud").is_none());
        assert!(TriageStatus::parse("pass").is_none());
        assert_eq!(TriageStatus::parse("New"), Some(TriageStatus::New));
        assert_eq!(
            TriageStatus::parse("Reviewing"),
            Some(TriageStatus::Reviewing)
        );
        assert_eq!(
            TriageStatus::parse("Confirmed"),
            Some(TriageStatus::Confirmed)
        );
        assert_eq!(
            TriageStatus::parse("Dismissed"),
            Some(TriageStatus::Dismissed)
        );
        assert_eq!(TriageStatus::Confirmed.as_str(), "Confirmed");
    }

    #[tokio::test]
    async fn triage_status_is_stored_and_emits_a_review_event() {
        let store = test_store().await;
        let verdict = Verdict {
            id: Uuid::new_v4(),
            recorded_at: timestamp(),
            outcome: VerdictOutcome::Violate,
            spec_id: "ledger.bill.no-second-payment".to_owned(),
            reason_code: Some("second-payment".to_owned()),
            evidence: serde_json::json!([{"role": "second-payment"}]),
        };
        store
            .insert_verdict(&verdict)
            .await
            .expect("insert verdict");

        let unknown = store
            .record_triage(Uuid::new_v4(), TriageStatus::Dismissed, None, "me")
            .await
            .expect("unknown verdict");
        assert!(matches!(unknown, TriageWrite::UnknownVerdict));

        let stored = store
            .record_triage(verdict.id, TriageStatus::Confirmed, Some("me"), "me")
            .await
            .expect("record triage");
        let TriageWrite::Stored(change) = stored else {
            panic!("verdict should be stored");
        };
        assert_eq!(change.triage.status, TriageStatus::Confirmed);
        assert_eq!(change.triage.assignee.as_deref(), Some("me"));
        assert_eq!(change.triage.updated_by, "me");

        let event = store
            .fetch_event(change.event_id)
            .await
            .expect("fetch review event")
            .expect("review event");
        assert_eq!(event.source, "inbox");
        assert_eq!(event.kind, REVIEW_EVENT_KIND);
        assert_eq!(event.data_class, "tenant");
        assert_eq!(event.payload["verdict_id"], verdict.id.to_string());
        assert_eq!(event.payload["status"], "Confirmed");
        assert_eq!(event.payload["actor"], "me");
        assert_eq!(event.payload["assignee"], "me");

        let again = store
            .record_triage(verdict.id, TriageStatus::Dismissed, None, "me")
            .await
            .expect("replace triage");
        let TriageWrite::Stored(replaced) = again else {
            panic!("replacement should be stored");
        };
        assert_eq!(replaced.triage.status, TriageStatus::Dismissed);
        assert!(replaced.triage.assignee.is_none());

        let listed = store.list_triage().await.expect("list triage");
        let row = listed
            .iter()
            .find(|item| item.verdict_id == verdict.id)
            .expect("listed row");
        assert_eq!(row.status, TriageStatus::Dismissed);
    }

    #[tokio::test]
    async fn rank_round_trip_keeps_unknown_dimensions_null() {
        let store = test_store().await;
        let verdict = Verdict {
            id: Uuid::new_v4(),
            recorded_at: timestamp(),
            outcome: VerdictOutcome::Violate,
            spec_id: "ledger.bill.no-second-payment".to_owned(),
            reason_code: Some("second-payment".to_owned()),
            evidence: serde_json::json!({ "title": "needs review" }),
        };
        store
            .insert_verdict(&verdict)
            .await
            .expect("insert verdict");
        let rank = VerdictRank {
            verdict_id: verdict.id,
            recorded_at: timestamp(),
            request: serde_json::json!({ "model": "jev-latest" }),
            response: None,
            money: Some(4.0),
            evidence: None,
            ambiguity: Some(0.0),
            intent: Some(2.0),
        };
        assert!(store
            .insert_rank_if_absent(&rank)
            .await
            .expect("insert rank"));
        assert!(!store
            .insert_rank_if_absent(&rank)
            .await
            .expect("repeat rank"));
        let loaded = store
            .list_ranks()
            .await
            .expect("list ranks")
            .into_iter()
            .find(|row| row.verdict_id == verdict.id)
            .expect("stored rank");
        assert_eq!(loaded.request, rank.request);
        assert!(loaded.response.is_none());
        assert_eq!(loaded.money, Some(4.0));
        assert!(loaded.evidence.is_none());
        assert_eq!(loaded.ambiguity, Some(0.0));
        assert_eq!(loaded.intent, Some(2.0));
        assert!(!format!("{loaded:?}").to_lowercase().contains("fraud"));
    }
    #[test]
    fn calibration_sql_reads_review_events_and_keeps_the_threshold() {
        let sql = include_str!("../migrations/0003_jev_calibration.sql");
        assert!(sql.contains(&format!("'{REVIEW_EVENT_KIND}'")));
        assert!(sql.contains(&format!("'{JUDGMENT_EVENT_KIND}'")));
        assert!(sql.contains(">= 0.80"));
        assert!(sql.contains("CREATE VIEW jev_calibration"));
        assert!(sql.contains("'Confirmed'"));
        assert!(sql.contains("'Dismissed'"));
        assert!(sql.contains("'unknown'"));
        assert!(!sql.to_ascii_lowercase().contains("fraud"));
    }

    #[tokio::test]
    async fn calibration_rejects_a_blank_question_and_an_unknown_verdict() {
        let store = test_store().await;
        let blank = store
            .record_jev_judgment(Uuid::new_v4(), "  ", Some(0.91))
            .await;
        assert!(matches!(blank, Err(StoreError::EmptyQuestion)));

        let missing = store
            .record_jev_judgment(Uuid::new_v4(), "same_obligation", Some(0.91))
            .await
            .expect("unknown verdict");
        assert_eq!(missing, JudgmentWrite::UnknownVerdict);
    }

    #[tokio::test]
    async fn calibration_counts_match_stored_review_events() {
        let store = test_store().await;
        let question = format!("cal-{}", Uuid::new_v4());
        let other = format!("cal-other-{}", Uuid::new_v4());
        let lonely = format!("cal-none-{}", Uuid::new_v4());

        let low = open_verdict(&store, VerdictOutcome::Violate).await;
        let below = open_verdict(&store, VerdictOutcome::Violate).await;
        let threshold = open_verdict(&store, VerdictOutcome::Violate).await;
        let mid = open_verdict(&store, VerdictOutcome::Violate).await;
        let high = open_verdict(&store, VerdictOutcome::Violate).await;
        let top = open_verdict(&store, VerdictOutcome::Violate).await;
        let replaced = open_verdict(&store, VerdictOutcome::Violate).await;
        let missing = open_verdict(&store, VerdictOutcome::Inconclusive).await;
        let unusable = open_verdict(&store, VerdictOutcome::Violate).await;
        let not_a_number = open_verdict(&store, VerdictOutcome::Violate).await;
        let direct = open_verdict(&store, VerdictOutcome::Violate).await;
        let unreviewed = open_verdict(&store, VerdictOutcome::Violate).await;
        let reviewing = open_verdict(&store, VerdictOutcome::Violate).await;
        let reopened = open_verdict(&store, VerdictOutcome::Violate).await;
        let ignored_fraud = open_verdict(&store, VerdictOutcome::Violate).await;
        let ignored_pass = open_verdict(&store, VerdictOutcome::Violate).await;
        let review_only = open_verdict(&store, VerdictOutcome::Violate).await;
        let second = open_verdict(&store, VerdictOutcome::Violate).await;
        let alone = open_verdict(&store, VerdictOutcome::Violate).await;

        judge(&store, low, &question, Some(0.0)).await;
        judge(&store, below, &question, Some(0.79)).await;
        judge(&store, threshold, &question, Some(0.80)).await;
        judge(&store, mid, &question, Some(0.85)).await;
        judge(&store, high, &question, Some(0.91)).await;
        judge(&store, top, &question, Some(1.0)).await;
        judge(&store, replaced, &question, Some(0.55)).await;
        judge(&store, missing, &question, None).await;
        judge(&store, unusable, &question, Some(1.2)).await;
        judge(&store, not_a_number, &question, Some(f64::NAN)).await;
        judge(&store, direct, &question, Some(0.41)).await;
        judge(&store, unreviewed, &question, Some(0.15)).await;
        judge(&store, reviewing, &question, Some(0.85)).await;
        judge(&store, reopened, &question, Some(0.85)).await;
        judge(&store, ignored_fraud, &question, Some(0.85)).await;
        judge(&store, ignored_pass, &question, Some(0.99)).await;
        judge(&store, second, &other, Some(0.95)).await;
        judge(&store, alone, &lonely, Some(0.85)).await;

        review(&store, low, TriageStatus::Confirmed).await;
        review(&store, below, TriageStatus::Confirmed).await;
        review(&store, threshold, TriageStatus::Confirmed).await;
        review(&store, mid, TriageStatus::Confirmed).await;
        review(&store, high, TriageStatus::Dismissed).await;
        review(&store, top, TriageStatus::Dismissed).await;
        review(&store, missing, TriageStatus::Confirmed).await;
        review(&store, unusable, TriageStatus::Confirmed).await;
        review(&store, not_a_number, TriageStatus::Confirmed).await;
        review(&store, reviewing, TriageStatus::Reviewing).await;
        review(&store, second, TriageStatus::Confirmed).await;
        review(&store, review_only, TriageStatus::Confirmed).await;

        review(&store, replaced, TriageStatus::Confirmed).await;
        later().await;
        judge(&store, replaced, &question, Some(0.85)).await;
        later().await;
        review(&store, replaced, TriageStatus::Dismissed).await;

        review(&store, reopened, TriageStatus::Confirmed).await;
        later().await;
        review(&store, reopened, TriageStatus::Reviewing).await;

        let event_id = review_event(&store, direct, "Confirmed").await;
        review_event(&store, ignored_fraud, "fraud").await;
        review_event(&store, ignored_pass, "pass").await;

        let event = store
            .fetch_event(event_id)
            .await
            .expect("read stored review")
            .expect("review event");
        assert_eq!(event.kind, REVIEW_EVENT_KIND);
        assert_eq!(event.source, "inbox");
        assert_eq!(event.payload["status"], "Confirmed");
        assert_eq!(event.payload["verdict_id"], direct.to_string());

        store.calibration().await.expect("read calibration");
        let mut rows = store
            .calibration_for(&question)
            .await
            .expect("read question rows");
        rows.extend(
            store
                .calibration_for(&other)
                .await
                .expect("read other question"),
        );
        rows.extend(
            store
                .calibration_for(&lonely)
                .await
                .expect("read lonely question"),
        );
        for row in rows.iter().filter(|row| row.question == question) {
            assert_eq!(row.reviewed, row.confirmed + row.dismissed);
            assert_ne!(row.band, "pass");
        }

        expect_counts(band(&rows, &question, "0.00-0.10"), 1, 0);
        expect_counts(band(&rows, &question, "0.70-0.80"), 1, 0);
        expect_counts(band(&rows, &question, "0.80-0.90"), 2, 1);
        expect_counts(band(&rows, &question, "0.90-1.00"), 0, 2);
        expect_counts(band(&rows, &question, "unknown"), 3, 0);
        expect_counts(band(&rows, &question, "0.40-0.50"), 1, 0);
        expect_counts(band(&rows, &other, "0.90-1.00"), 1, 0);

        absent(&rows, &question, "0.10-0.20");
        absent(&rows, &question, "0.50-0.60");
        absent(&rows, &lonely, "0.80-0.90");
        assert!(rows.iter().all(|row| row.question != lonely));

        let stored = store
            .fetch_verdict(missing)
            .await
            .expect("fetch verdict")
            .expect("inconclusive verdict");
        assert_eq!(stored.outcome, VerdictOutcome::Inconclusive);
    }

    async fn open_verdict(store: &Store, outcome: VerdictOutcome) -> Uuid {
        let id = Uuid::new_v4();
        let title = match outcome {
            VerdictOutcome::Violate => "needs review",
            VerdictOutcome::Satisfy => "satisfied",
            VerdictOutcome::Inconclusive => "inconclusive",
        };
        store
            .insert_verdict(&Verdict {
                id,
                recorded_at: timestamp(),
                outcome,
                spec_id: "ledger.bill.no-second-payment".to_owned(),
                reason_code: Some("second-payment".to_owned()),
                evidence: serde_json::json!({
                    "title": title,
                    "binding": "bill",
                    "evidence": "second-payment",
                    "timeline": []
                }),
            })
            .await
            .expect("insert verdict");
        id
    }

    async fn judge(store: &Store, verdict_id: Uuid, question: &str, probability: Option<f64>) {
        let write = store
            .record_jev_judgment(verdict_id, question, probability)
            .await
            .expect("store judgment");
        assert!(matches!(write, JudgmentWrite::Stored { .. }));
    }

    async fn review(store: &Store, verdict_id: Uuid, status: TriageStatus) {
        let write = store
            .record_triage(verdict_id, status, None, "me")
            .await
            .expect("store review");
        assert!(matches!(write, TriageWrite::Stored(_)));
    }

    async fn review_event(store: &Store, verdict_id: Uuid, status: &str) -> Uuid {
        let id = Uuid::new_v4();
        store
            .insert_event(&Event {
                id,
                observed_at: Utc::now(),
                source: "inbox".to_owned(),
                kind: REVIEW_EVENT_KIND.to_owned(),
                data_class: "tenant".to_owned(),
                payload: serde_json::json!({
                    "verdict_id": verdict_id.to_string(),
                    "status": status,
                    "assignee": serde_json::Value::Null,
                    "actor": "me",
                }),
            })
            .await
            .expect("insert review event");
        id
    }

    async fn later() {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    fn band<'a>(rows: &'a [CalibrationRow], question: &str, band: &str) -> &'a CalibrationRow {
        rows.iter()
            .find(|row| row.question == question && row.band == band)
            .unwrap_or_else(|| panic!("missing {question} {band}"))
    }

    fn expect_counts(row: &CalibrationRow, confirmed: i64, dismissed: i64) {
        assert_eq!(row.confirmed, confirmed, "{}", row.band);
        assert_eq!(row.dismissed, dismissed, "{}", row.band);
        assert_eq!(row.reviewed, confirmed + dismissed);
    }

    fn absent(rows: &[CalibrationRow], question: &str, band: &str) {
        assert!(
            rows.iter()
                .all(|row| row.question != question || row.band != band),
            "{question} {band} should be absent"
        );
    }
}
