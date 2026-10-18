-- Local TimescaleDB schema for live ledger events and verdicts.
-- Hypertable adapted from
-- https://docs.timescale.com/use-timescale/latest/hypertables/create/
--
-- `events` is partitioned on observed_at so later time-window queries have a
-- time column. `verdicts` is a regular table.
-- Timescale requires the partition column in every unique index, so the events
-- primary key is (id, observed_at). Callers still treat `id` as the event
-- identity; reads return the latest row for that id.
-- `payload` and `evidence` are jsonb so later issues can attach attributes
-- without a second model. Verdict outcomes are Satisfy, Violate, or
-- Inconclusive.

CREATE EXTENSION IF NOT EXISTS timescaledb;

CREATE TABLE events (
    id uuid NOT NULL,
    observed_at timestamptz NOT NULL,
    source text NOT NULL,
    kind text NOT NULL,
    data_class text NOT NULL,
    payload jsonb NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY (id, observed_at)
);

-- PERFORM yields no client row, which the migration runner can execute.
DO $$
BEGIN
    PERFORM create_hypertable(
        'events',
        by_range('observed_at'),
        if_not_exists => TRUE
    );
END
$$;

CREATE TABLE verdicts (
    id uuid PRIMARY KEY,
    recorded_at timestamptz NOT NULL,
    outcome text NOT NULL,
    spec_id text NOT NULL,
    reason_code text,
    evidence jsonb NOT NULL DEFAULT '[]'::jsonb,
    CONSTRAINT verdicts_outcome_known CHECK (
        outcome IN ('Satisfy', 'Violate', 'Inconclusive')
    )
);
