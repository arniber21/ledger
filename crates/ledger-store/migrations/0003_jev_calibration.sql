-- Calibration of stored Jev scores against inbox reviews.
--
-- Labels are the latest verdict.review event for each verdict. Only Confirmed
-- and Dismissed are labels. New and Reviewing are not. Each label joins the
-- latest jev.judgment event for that verdict and question.
--
-- The score is the stored probability. A missing, non-numeric, or out-of-range
-- probability is the unknown band. Unknown is inconclusive, not a pass.
-- Reviews with no stored judgment are omitted. This view does not invent a
-- question or a score for them.
--
-- Bands are 0.10 wide. The 0.80 edge is SAME_PROBABILITY_THRESHOLD and
-- DIFFERENT_PROBABILITY_THRESHOLD in ledger-jev. This view does not change
-- those thresholds. Counts are the sample. A small reviewed count is not a rate.

CREATE VIEW jev_calibration AS
WITH latest_review AS (
    SELECT DISTINCT ON (payload->>'verdict_id')
        payload->>'verdict_id' AS verdict_id,
        payload->>'status' AS status
    FROM events
    WHERE kind = 'verdict.review'
      AND payload->>'verdict_id' IS NOT NULL
    ORDER BY payload->>'verdict_id', observed_at DESC, id DESC
),
labeled AS (
    SELECT verdict_id, status
    FROM latest_review
    WHERE status IN ('Confirmed', 'Dismissed')
),
latest_judgment AS (
    SELECT DISTINCT ON (payload->>'verdict_id', payload->>'question')
        payload->>'verdict_id' AS verdict_id,
        payload->>'question' AS question,
        CASE
            WHEN jsonb_typeof(payload->'probability') = 'number'
             AND (payload->>'probability')::double precision >= 0
             AND (payload->>'probability')::double precision <= 1
            THEN (payload->>'probability')::double precision
            ELSE NULL
        END AS probability
    FROM events
    WHERE kind = 'jev.judgment'
      AND payload->>'verdict_id' IS NOT NULL
      AND payload->>'question' IS NOT NULL
      AND btrim(payload->>'question') <> ''
    ORDER BY payload->>'verdict_id', payload->>'question', observed_at DESC, id DESC
),
banded AS (
    SELECT
        j.question,
        CASE
            WHEN j.probability IS NULL THEN 'unknown'
            WHEN j.probability >= 0.90 THEN '0.90-1.00'
            WHEN j.probability >= 0.80 THEN '0.80-0.90'
            WHEN j.probability >= 0.70 THEN '0.70-0.80'
            WHEN j.probability >= 0.60 THEN '0.60-0.70'
            WHEN j.probability >= 0.50 THEN '0.50-0.60'
            WHEN j.probability >= 0.40 THEN '0.40-0.50'
            WHEN j.probability >= 0.30 THEN '0.30-0.40'
            WHEN j.probability >= 0.20 THEN '0.20-0.30'
            WHEN j.probability >= 0.10 THEN '0.10-0.20'
            ELSE '0.00-0.10'
        END AS band,
        r.status
    FROM latest_judgment AS j
    INNER JOIN labeled AS r ON r.verdict_id = j.verdict_id
)
SELECT
    question,
    band,
    count(*) FILTER (WHERE status = 'Confirmed')::bigint AS confirmed,
    count(*) FILTER (WHERE status = 'Dismissed')::bigint AS dismissed,
    count(*)::bigint AS reviewed
FROM banded
GROUP BY question, band
ORDER BY question, band;
