-- Triage status lives next to `verdicts`. A status change also inserts a
-- `verdict.review` row into the `events` hypertable so the audit trail shows
-- who reviewed what. Status is New, Reviewing, Confirmed, or Dismissed.

CREATE TABLE verdict_triage (
    verdict_id uuid PRIMARY KEY REFERENCES verdicts (id),
    status text NOT NULL,
    assignee text,
    updated_at timestamptz NOT NULL,
    updated_by text NOT NULL,
    CONSTRAINT verdict_triage_status_known CHECK (
        status IN ('New', 'Reviewing', 'Confirmed', 'Dismissed')
    )
);
