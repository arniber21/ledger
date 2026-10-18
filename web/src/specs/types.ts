/**
 * Spec authoring view state.
 *
 * Steps come from `ledger-spec`'s SSE frames (`sentence`, `draft`, `check`,
 * `accepted`, `rejected`). The check frame carries a verdict only. Scores are
 * not invented. The 0.80 figure is the fixed Jev threshold in `ledger-jev`
 * (`SAME_PROBABILITY_THRESHOLD` and `DIFFERENT_PROBABILITY_THRESHOLD`).
 */

export const JEV_THRESHOLD_LABEL = "0.80";

/** Drafter named by the authoring pipeline, recorded only after a draft frame. */
export const DRAFTER = "Gemini";

export type MatchVerdict = "Same" | "Different" | "Unknown";

export type SpecStatus = "composing" | "drafting" | "armed" | "rejected" | "unavailable";

/** Clause marks from the stream. Absent means coverage was not reported. */
export type ClauseMark = {
  text: string;
  coverage: "covered" | "uncovered" | "unknown";
};

export type TimelineStep =
  | { kind: "policy"; text: string }
  | { kind: "draft"; patternId: string; lines: string[] }
  | { kind: "catalogue"; ok: boolean; detail: string }
  | { kind: "check"; verdict: MatchVerdict }
  | { kind: "armed"; patternId: string; monitor: string }
  | { kind: "monitor"; patternId: string; monitor: string }
  | { kind: "rejected"; reason: string; detail: string };

export type SpecRun = {
  key: string;
  /** `ledger.authored.<pattern>` once the stream accepts the draft. */
  specId: string | null;
  sentence: string;
  clauses: ClauseMark[] | null;
  patternId: string | null;
  parameters: Record<string, string | number>;
  status: SpecStatus;
  match: MatchVerdict | null;
  rejectReason: string | null;
  monitor: string | null;
  draftedBy: string | null;
  steps: TimelineStep[];
  /** Connection or stream failure. Not a model verdict. */
  error: string | null;
};

export type AuthorEvent =
  | { event: "sentence"; text: string; clauses: ClauseMark[] | null }
  | { event: "draft"; patternId: string; parameters: Record<string, string | number> }
  | { event: "check"; verdict: MatchVerdict }
  | { event: "accepted"; patternId: string; monitor: string }
  | {
      event: "rejected";
      reason: string;
      code: string | null;
      parameter: string | null;
      eventKind: string | null;
    };
