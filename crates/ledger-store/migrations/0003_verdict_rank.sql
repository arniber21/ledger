-- Raw Jev Score results for inbox ranking. One row per verdict.
-- NULL on a dimension means Unknown: the call failed, contradicted itself,
-- or the level probability was under the fixed 0.80 threshold. NULL is not a
-- pass and not a priority of zero. `request` and `response` are the stored
-- System One input and output. They contain no API key.

CREATE TABLE verdict_rank (
    verdict_id uuid PRIMARY KEY REFERENCES verdicts (id),
    recorded_at timestamptz NOT NULL,
    request jsonb NOT NULL,
    response jsonb,
    money double precision,
    evidence double precision,
    ambiguity double precision,
    intent double precision
);
