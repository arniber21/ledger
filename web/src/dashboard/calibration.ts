import { JEV_THRESHOLD_LABEL } from "../specs/types";

/** Reviewed counts below this are marked so they are not read as a rate. */
export const SMALL_SAMPLE = 10;

export type CalibrationRow = {
  question: string;
  band: string;
  confirmed: number;
  dismissed: number;
  reviewed: number;
};

export type CalibrationTable = {
  thresholdLabel: string;
  rows: CalibrationRow[];
};

const countFormat = new Intl.NumberFormat("en-US");

export function countLabel(value: number): string {
  return countFormat.format(value);
}

/** Score band for the table. `unknown` is inconclusive, not a pass. */
export function bandLabel(band: string): string {
  if (band === "unknown") return "Unknown · inconclusive";
  const match = /^(\d\.\d\d)-(\d\.\d\d)$/.exec(band);
  if (!match) return band;
  return `${match[1]}–${match[2]}`;
}

const QUESTION_NAMES: Record<string, string> = {
  same_obligation: "Same obligation",
  memo_instructs_payment: "Memo instructs payment",
  draft_matches_policy: "Draft matches policy",
};

const OPAQUE_ID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

/**
 * Test and seed rows use ids such as `cal-<uuid>` and `cal-other-<uuid>`.
 * They are not Jev questions a person asked, so the table leaves them out.
 */
export function isSeedQuestion(question: string): boolean {
  return question.startsWith("cal-") || OPAQUE_ID.test(question);
}

/** A readable name for a stored question id. Unknown ids become sentence case. */
export function questionLabel(question: string): string {
  const known = QUESTION_NAMES[question];
  if (known) return known;
  const words = question.replace(/[_.-]+/g, " ").trim();
  return words.length === 0 ? question : `${words.charAt(0).toUpperCase()}${words.slice(1)}`;
}

export type CalibrationGroup = {
  question: string;
  label: string;
  reviewed: number;
  rows: CalibrationRow[];
};

/** Rows grouped by question, in the order the API sent them, seeds removed. */
export function groupCalibration(rows: readonly CalibrationRow[]): CalibrationGroup[] {
  const groups = new Map<string, CalibrationGroup>();
  for (const row of rows) {
    if (isSeedQuestion(row.question)) continue;
    const group = groups.get(row.question) ?? {
      question: row.question,
      label: questionLabel(row.question),
      reviewed: 0,
      rows: [],
    };
    group.rows.push(row);
    group.reviewed += row.reviewed;
    groups.set(row.question, group);
  }
  return [...groups.values()];
}

export function reviewedLabel(reviewed: number): string {
  const count = countLabel(reviewed);
  if (reviewed < SMALL_SAMPLE) return `${count} · small sample`;
  return count;
}

export function thresholdLabel(value: unknown): string {
  if (typeof value !== "number" || !Number.isFinite(value)) return JEV_THRESHOLD_LABEL;
  return value.toFixed(2);
}

export function parseCalibration(payload: unknown): CalibrationTable | null {
  if (!payload || typeof payload !== "object") return null;
  const body = payload as Record<string, unknown>;
  if (!Array.isArray(body.rows)) return null;
  const rows: CalibrationRow[] = [];
  for (const item of body.rows) {
    const row = rowFrom(item);
    if (row) rows.push(row);
  }
  return { thresholdLabel: thresholdLabel(body.threshold), rows };
}

function rowFrom(value: unknown): CalibrationRow | null {
  if (!value || typeof value !== "object") return null;
  const row = value as Record<string, unknown>;
  if (typeof row.question !== "string" || row.question.trim() === "") return null;
  if (typeof row.band !== "string" || row.band.trim() === "") return null;
  const confirmed = count(row.confirmed);
  const dismissed = count(row.dismissed);
  const reviewed = count(row.reviewed);
  if (confirmed == null || dismissed == null || reviewed == null) return null;
  return { question: row.question, band: row.band, confirmed, dismissed, reviewed };
}

function count(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0) return null;
  return value;
}
