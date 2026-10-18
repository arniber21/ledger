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
